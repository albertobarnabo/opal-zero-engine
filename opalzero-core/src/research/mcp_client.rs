//! Minimal MCP client for consuming external MCP servers as research sources.
//!
//! Supports two transports, selected by the `server` string prefix:
//!
//! | Prefix | Transport |
//! |--------|-----------|
//! | `http://` or `https://` | HTTP JSON-RPC POST (synchronous) |
//! | `stdio:` | Subprocess — newline-delimited JSON-RPC over stdin/stdout |
//!
//! Both transports implement the minimal MCP sequence needed to call one tool:
//!   initialize → (notifications/initialized) → tools/call → result

use serde_json::Value;
use std::time::Duration;

const MCP_TIMEOUT_SECS: u64 = 30;

// ── Public entry point ────────────────────────────────────────────────────────

/// Call `tool_name` on the MCP server described by `server` and return the
/// text content of the response.
///
/// - `server`: `https://host/path`, `http://host/path`, or `stdio:cmd [args…]`
/// - `arguments`: JSON object forwarded as-is to the tool
/// - `api_key`: sent as `Authorization: Bearer …` for HTTP, ignored for stdio
pub async fn call_mcp_tool(
    server:    &str,
    tool_name: &str,
    arguments: Value,
    api_key:   Option<&str>,
) -> Result<String, String> {
    tokio::time::timeout(
        Duration::from_secs(MCP_TIMEOUT_SECS),
        dispatch(server, tool_name, arguments, api_key),
    )
    .await
    .map_err(|_| format!("MCP call to '{server}' timed out after {MCP_TIMEOUT_SECS}s"))?
}

async fn dispatch(
    server:    &str,
    tool_name: &str,
    arguments: Value,
    api_key:   Option<&str>,
) -> Result<String, String> {
    if server.starts_with("http://") || server.starts_with("https://") {
        call_via_http(server, tool_name, arguments, api_key).await
    } else if let Some(cmd) = server.strip_prefix("stdio:") {
        call_via_stdio(cmd.trim(), tool_name, arguments).await
    } else {
        Err(format!(
            "Unsupported MCP server scheme in '{server}'. \
             Use 'https://...', 'http://...', or 'stdio:command [args]'."
        ))
    }
}

// ── HTTP transport ────────────────────────────────────────────────────────────

async fn call_via_http(
    url:       &str,
    tool_name: &str,
    arguments: Value,
    api_key:   Option<&str>,
) -> Result<String, String> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments,
        }
    });

    let client = reqwest::Client::new();
    let mut builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&payload);

    if let Some(key) = api_key {
        builder = builder.header("Authorization", format!("Bearer {key}"));
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("MCP HTTP request to '{url}' failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("MCP server '{url}' returned HTTP {status}: {body}"));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("MCP response from '{url}' is not valid JSON: {e}"))?;

    extract_mcp_result(&body, url)
}

// ── Stdio transport ───────────────────────────────────────────────────────────

async fn call_via_stdio(
    command_str: &str,
    tool_name:   &str,
    arguments:   Value,
) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;

    // Parse "npx @exa-ai/exa-mcp --flag" → cmd="npx", args=["@exa-ai/exa-mcp", "--flag"]
    let mut parts = command_str.split_whitespace();
    let cmd = parts
        .next()
        .ok_or_else(|| "MCP stdio: empty command string".to_string())?;
    let args: Vec<&str> = parts.collect();

    let mut child = Command::new(cmd)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| {
            format!(
                "MCP stdio: failed to spawn '{command_str}': {e}. \
                 Make sure the server is installed and on PATH."
            )
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or("MCP stdio: could not open stdin")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("MCP stdio: could not open stdout")?;
    let mut reader = BufReader::new(stdout);
    let mut line   = String::new();

    // ── 1. Send initialize ────────────────────────────────────────────────────
    write_jsonrpc(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "opalzero-research", "version": "1.0" }
            }
        }),
    )
    .await?;

    // Read initialize response (we don't validate it, just consume it)
    line.clear();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("MCP stdio read error: {e}"))?;

    // ── 2. Send notifications/initialized ─────────────────────────────────────
    write_jsonrpc(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )
    .await?;

    // ── 3. Send tools/call ────────────────────────────────────────────────────
    write_jsonrpc(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": tool_name, "arguments": arguments }
        }),
    )
    .await?;

    stdin
        .flush()
        .await
        .map_err(|e| format!("MCP stdio flush error: {e}"))?;

    // ── 4. Read response — skip any notifications, return first result ─────────
    let result = loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("MCP stdio read error: {e}"))?;
        if n == 0 {
            let _ = child.kill().await;
            return Err(format!(
                "MCP stdio server '{command_str}' closed before sending a response"
            ));
        }
        let val: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("MCP stdio: could not parse response line: {e}"))?;
        // Notifications have no "id" field — skip them.
        if val.get("id").is_some() {
            break val;
        }
    };

    let _ = child.kill().await;
    extract_mcp_result(&result, command_str)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn write_jsonrpc(
    stdin: &mut tokio::process::ChildStdin,
    msg:   &Value,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let bytes = serde_json::to_string(msg)
        .map_err(|e| format!("MCP: failed to serialize message: {e}"))?;
    stdin
        .write_all(bytes.as_bytes())
        .await
        .map_err(|e| format!("MCP stdio write error: {e}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|e| format!("MCP stdio write error: {e}"))
}

/// Extract text content from an MCP JSON-RPC response.
///
/// MCP success format:
/// ```json
/// {"result": {"content": [{"type": "text", "text": "..."}]}}
/// ```
fn extract_mcp_result(body: &Value, source: &str) -> Result<String, String> {
    // JSON-RPC error
    if let Some(err) = body.get("error") {
        let msg = err["message"].as_str().unwrap_or("unknown error");
        let code = err["code"].as_i64().unwrap_or(0);
        return Err(format!("MCP server '{source}' returned error {code}: {msg}"));
    }

    let result = body
        .get("result")
        .ok_or_else(|| format!("MCP response from '{source}' missing 'result' field"))?;

    // Standard MCP content array
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        let text: Vec<&str> = content
            .iter()
            .filter_map(|item| {
                if item["type"].as_str() == Some("text") {
                    item["text"].as_str()
                } else {
                    None
                }
            })
            .collect();
        if !text.is_empty() {
            return Ok(text.join("\n"));
        }
    }

    // Fallback: serialize the whole result
    serde_json::to_string_pretty(result)
        .map_err(|e| format!("MCP: could not serialize result: {e}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_result_parses_text_content() {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [
                    { "type": "text", "text": "Hello from MCP" },
                    { "type": "text", "text": "Second block" }
                ]
            }
        });
        let result = extract_mcp_result(&body, "test").unwrap();
        assert_eq!(result, "Hello from MCP\nSecond block");
    }

    #[test]
    fn extract_result_returns_error_on_jsonrpc_error() {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": "Tool not found" }
        });
        let err = extract_mcp_result(&body, "test").unwrap_err();
        assert!(err.contains("Tool not found"));
    }

    #[test]
    fn extract_result_missing_result_field_is_error() {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1 });
        assert!(extract_mcp_result(&body, "test").is_err());
    }

    #[test]
    fn dispatch_rejects_unknown_scheme() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let err = rt.block_on(async {
            dispatch("ws://localhost:3000", "tool", serde_json::json!({}), None).await
        });
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("Unsupported MCP server scheme"));
    }
}
