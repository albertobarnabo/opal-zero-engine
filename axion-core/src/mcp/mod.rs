//! # Axion MCP Server
//!
//! Exposes Axion's tool set as a **Model Context Protocol (MCP) server**,
//! allowing any MCP-compatible AI agent (Claude Code, Gemini CLI, Codex, …)
//! to discover and call Axion tools without configuration.
//!
//! ## Protocol
//!
//! MCP is JSON-RPC 2.0 carried over **stdin/stdout** (newline-delimited).
//! The server implements the three methods required by the MCP 2025-11-25 spec:
//!
//! | Method | Purpose |
//! |--------|---------|
//! | `initialize` | Protocol handshake; returns server capabilities |
//! | `tools/list` | Returns all exposed Axion tools in MCP schema format |
//! | `tools/call` | Executes a tool and returns the result |
//!
//! Notifications (`notifications/initialized`) are accepted and silently discarded.
//!
//! ## Tools exposed
//!
//! All general-purpose Axion tools are exposed.  Mission-internal tools that
//! have no meaning outside an active Axion mission loop are excluded:
//!
//! | Excluded | Reason |
//! |----------|--------|
//! | `finalize_mission_state` | Axion-internal: terminates a mission, meaningless standalone |
//! | `feedback` | Requires a live mission HITL loop to receive the response |
//! | `build_dynamic_ui` | Requires an active mission state to attach the UI to |
//! | `vision` | Requires a pre-configured `uploads/` WASI preopen; complex for external callers |
//!
//! ## Running the server
//!
//! ```bash
//! axion mcp serve
//! ```
//!
//! Then register it in Claude Code (project-level):
//!
//! ```bash
//! claude mcp add axion -- axion mcp serve
//! ```
//!
//! After this, every Claude Code session in the project automatically has access
//! to all Axion tools (`web_search`, `calculator`, `sqlite_query`, …).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::protocol::Tool;
use crate::tools::RequestKeys;

// ── JSON-RPC 2.0 wire types ───────────────────────────────────────────────────

/// An incoming JSON-RPC 2.0 message (request or notification).
#[derive(Debug, Deserialize)]
struct RpcIn {
    /// Absent on notifications.
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

/// A successful JSON-RPC 2.0 response.
#[derive(Serialize)]
struct RpcOk {
    jsonrpc: &'static str,
    id: Value,
    result: Value,
}

/// An error JSON-RPC 2.0 response.
#[derive(Serialize)]
struct RpcErr {
    jsonrpc: &'static str,
    id: Value,
    error: RpcErrBody,
}

#[derive(Serialize)]
struct RpcErrBody {
    code: i32,
    message: String,
}

impl RpcOk {
    fn new(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result }
    }
}

impl RpcErr {
    fn new(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            error: RpcErrBody { code, message: message.into() },
        }
    }

    /// Method not found (-32601).
    fn method_not_found(id: Value, method: &str) -> Self {
        Self::new(id, -32601, format!("Method not found: {}", method))
    }

    /// Tool execution error (-32000 = server-defined).
    fn tool_error(id: Value, message: impl Into<String>) -> Self {
        Self::new(id, -32000, message)
    }
}

// ── MCP tool schema conversion ────────────────────────────────────────────────

/// Convert an Axion [`Tool`] to the MCP `inputSchema` JSON object.
///
/// Axion's `ToolParameters` is already a valid JSON Schema fragment; we just
/// re-serialize it under the `inputSchema` key that MCP expects.
fn to_mcp_tool(tool: &Tool) -> Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {
            "type": "object",
            "properties": serde_json::to_value(&tool.parameters.properties).unwrap_or_default(),
            "required": tool.parameters.required,
        }
    })
}

// ── Exposed tool set ──────────────────────────────────────────────────────────

/// The Axion tools exposed over MCP.
///
/// Mission-internal tools (`finalize_mission_state`, `feedback`,
/// `build_dynamic_ui`, `vision`) are excluded — they require active mission
/// context and have no meaningful standalone semantics.
fn mcp_tools() -> Vec<Tool> {
    vec![
        Tool::web_search(),
        Tool::calculator(),
        Tool::fetch_page(),
        Tool::http_request(),
        Tool::sqlite_query(),
        Tool::python_interpreter(),
        Tool::read_csv(),
        Tool::rss_reader(),
        Tool::diff(),
        Tool::extract_pdf_text(),
        Tool::write_file(),
        Tool::send_email(),
        Tool::memory_persist(),
    ]
}

// ── AxionMcpServer ────────────────────────────────────────────────────────────

/// An MCP server that exposes Axion's tool set over stdin/stdout (JSON-RPC 2.0).
///
/// Each server instance carries a unique **session ID** used to scope
/// `sqlite_query` calls to an isolated per-session database, so state built
/// up across multiple tool calls within one session persists naturally.
///
/// # Usage
///
/// ```rust,no_run
/// use axion_core::mcp::AxionMcpServer;
///
/// #[tokio::main]
/// async fn main() {
///     AxionMcpServer::new().serve().await.unwrap();
/// }
/// ```
pub struct AxionMcpServer {
    /// Unique session ID — scopes per-session SQLite storage.
    session_id: String,
    /// Optional API key overrides (read from environment by default).
    keys: RequestKeys,
}

impl Default for AxionMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AxionMcpServer {
    /// Create a new server with a fresh session ID and default (env-var) keys.
    pub fn new() -> Self {
        Self {
            session_id: format!("mcp-{}", Uuid::new_v4()),
            keys: RequestKeys::default(),
        }
    }

    /// Run the MCP stdio server loop.
    ///
    /// Reads newline-delimited JSON-RPC messages from stdin, dispatches each
    /// one, and writes the response (if any) to stdout.  Returns when stdin
    /// reaches EOF (the MCP client disconnected).
    pub async fn serve(self) -> Result<(), String> {
        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        tracing::info!(session_id = %self.session_id, "Axion MCP server started");

        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("stdin read error: {}", e))?;

            if n == 0 {
                // EOF — client disconnected cleanly.
                tracing::info!("Axion MCP server: EOF on stdin — shutting down");
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let response_bytes = match serde_json::from_str::<RpcIn>(trimmed) {
                Err(e) => {
                    // Parse error (-32700).
                    let err = RpcErr::new(
                        Value::Null,
                        -32700,
                        format!("Parse error: {}", e),
                    );
                    serde_json::to_vec(&err).unwrap_or_default()
                }
                Ok(msg) => {
                    // Notifications (no `id`) get no response.
                    if msg.id.is_none() {
                        tracing::debug!(method = %msg.method, "MCP notification received");
                        continue;
                    }
                    let id = msg.id.unwrap();
                    match self.dispatch(id, &msg.method, msg.params).await {
                        Some(bytes) => bytes,
                        None => continue,
                    }
                }
            };

            stdout
                .write_all(&response_bytes)
                .await
                .map_err(|e| format!("stdout write error: {}", e))?;
            stdout
                .write_all(b"\n")
                .await
                .map_err(|e| format!("stdout write error: {}", e))?;
            stdout
                .flush()
                .await
                .map_err(|e| format!("stdout flush error: {}", e))?;
        }

        Ok(())
    }

    /// Route a JSON-RPC request to the appropriate handler.
    ///
    /// Returns `None` for notifications (which should not be responded to).
    async fn dispatch(&self, id: Value, method: &str, params: Value) -> Option<Vec<u8>> {
        let result = match method {
            "initialize" => self.handle_initialize(&params),
            "tools/list" => self.handle_tools_list(),
            "tools/call" => self.handle_tools_call(id.clone(), &params).await,
            // ping is commonly sent by MCP clients for keep-alive.
            "ping" => Ok(serde_json::json!({})),
            other => {
                let err = RpcErr::method_not_found(id, other);
                return Some(serde_json::to_vec(&err).unwrap_or_default());
            }
        };

        let bytes = match result {
            Ok(value) => {
                serde_json::to_vec(&RpcOk::new(id, value)).unwrap_or_default()
            }
            Err(msg) => {
                serde_json::to_vec(&RpcErr::tool_error(id, msg)).unwrap_or_default()
            }
        };
        Some(bytes)
    }

    // ── Handlers ──────────────────────────────────────────────────────────────

    fn handle_initialize(&self, _params: &Value) -> Result<Value, String> {
        Ok(serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "axion-mcp",
                "version": env!("CARGO_PKG_VERSION"),
            }
        }))
    }

    fn handle_tools_list(&self) -> Result<Value, String> {
        let tools: Vec<Value> = mcp_tools().iter().map(to_mcp_tool).collect();
        tracing::debug!(tool_count = tools.len(), "MCP tools/list requested");
        Ok(serde_json::json!({ "tools": tools }))
    }

    async fn handle_tools_call(&self, _id: Value, params: &Value) -> Result<Value, String> {
        let name = params["name"]
            .as_str()
            .ok_or_else(|| "tools/call: missing 'name' field".to_string())?;

        // Validate the tool is in our exposed set — reject calls to mission-
        // internal tools even if the caller somehow knows their names.
        let allowed: Vec<String> = mcp_tools().iter().map(|t| t.name.clone()).collect();
        if !allowed.contains(&name.to_string()) {
            return Err(format!(
                "Tool '{}' is not exposed via MCP. Available tools: {}",
                name,
                allowed.join(", ")
            ));
        }

        // MCP sends arguments as a JSON object; Axion's execute_tool expects
        // the same object serialized as a JSON string.
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let arguments_str = serde_json::to_string(&arguments)
            .map_err(|e| format!("Failed to serialize arguments: {}", e))?;

        tracing::info!(tool = name, "MCP tool call");

        match crate::tools::execute_tool(name, &arguments_str, &self.session_id, &self.keys).await
        {
            Ok(result) => {
                // MCP content format: array of content blocks.
                Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": result }]
                }))
            }
            Err(e) => {
                tracing::warn!(tool = name, error = %e, "MCP tool call failed");
                // Return an error response with isError flag so the agent knows
                // the call failed without crashing the session.
                Ok(serde_json::json!({
                    "content": [{ "type": "text", "text": format!("Tool error: {}", e) }],
                    "isError": true
                }))
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tools_excludes_mission_internal_tools() {
        let tools = mcp_tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        // These must NOT be present.
        assert!(!names.contains(&"finalize_mission_state"), "finalize_mission_state must be excluded");
        assert!(!names.contains(&"feedback"), "feedback must be excluded");
        assert!(!names.contains(&"build_dynamic_ui"), "build_dynamic_ui must be excluded");
        assert!(!names.contains(&"vision"), "vision must be excluded");
        // These MUST be present.
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"calculator"));
        assert!(names.contains(&"python_interpreter"));
        assert!(names.contains(&"sqlite_query"));
    }

    #[test]
    fn to_mcp_tool_produces_input_schema() {
        let tool = Tool::web_search();
        let mcp = to_mcp_tool(&tool);
        assert_eq!(mcp["name"], "web_search");
        assert!(mcp["description"].is_string());
        // MCP uses `inputSchema`, not `parameters`.
        assert!(mcp.get("inputSchema").is_some(), "must have inputSchema");
        assert!(mcp.get("parameters").is_none(), "must NOT have parameters");
        // inputSchema must have required array.
        assert!(mcp["inputSchema"]["required"].is_array());
    }

    #[test]
    fn to_mcp_tool_schema_has_properties() {
        let tool = Tool::calculator();
        let mcp = to_mcp_tool(&tool);
        let props = &mcp["inputSchema"]["properties"];
        assert!(props.is_object());
        assert!(props.get("operation").is_some());
        assert!(props.get("values").is_some());
    }

    #[test]
    fn axion_mcp_server_new_has_unique_session_ids() {
        let a = AxionMcpServer::new();
        let b = AxionMcpServer::new();
        assert_ne!(a.session_id, b.session_id);
        assert!(a.session_id.starts_with("mcp-"));
    }

    #[test]
    fn handle_initialize_returns_protocol_version() {
        let server = AxionMcpServer::new();
        let result = server.handle_initialize(&Value::Null).unwrap();
        assert_eq!(result["protocolVersion"], "2025-11-25");
        assert_eq!(result["serverInfo"]["name"], "axion-mcp");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn handle_tools_list_returns_correct_count() {
        let server = AxionMcpServer::new();
        let result = server.handle_tools_list().unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), mcp_tools().len());
        // Every tool must have name, description, and inputSchema.
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert!(t["inputSchema"].is_object());
        }
    }

    #[tokio::test]
    async fn handle_tools_call_rejects_mission_internal_tools() {
        let server = AxionMcpServer::new();
        let params = serde_json::json!({
            "name": "finalize_mission_state",
            "arguments": {}
        });
        let result = server
            .handle_tools_call(Value::Number(1.into()), &params)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not exposed via MCP"));
    }

    #[tokio::test]
    async fn handle_tools_call_missing_name_returns_error() {
        let server = AxionMcpServer::new();
        let result = server
            .handle_tools_call(Value::Number(1.into()), &serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }
}
