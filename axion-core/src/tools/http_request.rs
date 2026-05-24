//! `http_request` tool — make arbitrary HTTP requests to any URL.
//!
//! Gives agents a general-purpose HTTP escape hatch: GET/POST to REST APIs,
//! webhooks, or any internal endpoint without needing a dedicated integration.
//! Response bodies are capped at 512 KB so agents never stall on large files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MAX_RESPONSE_BYTES: usize = 512 * 1024; // 512 KB
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Deserialize)]
struct HttpRequestArgs {
    /// Full URL including scheme (https://...).
    url: String,
    /// HTTP method — GET, POST, PUT, PATCH, DELETE. Default: GET.
    #[serde(default = "default_method")]
    method: String,
    /// Optional request headers as key→value pairs.
    #[serde(default)]
    headers: HashMap<String, String>,
    /// Optional request body. For JSON APIs, serialize to a string first.
    #[serde(default)]
    body: Option<String>,
    /// Request timeout in seconds. Default: 30.
    #[serde(default = "default_timeout")]
    timeout_secs: u64,
}

fn default_method() -> String { "GET".to_string() }
fn default_timeout() -> u64   { DEFAULT_TIMEOUT_SECS }

#[derive(Serialize)]
struct HttpResponse {
    status:  u16,
    ok:      bool,
    headers: HashMap<String, String>,
    body:    String,
    /// True when the body was truncated to MAX_RESPONSE_BYTES.
    truncated: bool,
}

pub async fn execute_http_request(arguments: &str) -> Result<String, String> {
    let args: HttpRequestArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("http_request: invalid arguments: {e}"))?;

    // Validate scheme — only allow https and http.
    if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
        return Err(format!("http_request: URL must start with http:// or https://: {}", args.url));
    }

    let timeout = std::time::Duration::from_secs(args.timeout_secs.min(120));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("axion-agent/1.0")
        .build()
        .map_err(|e| format!("http_request: client build failed: {e}"))?;

    let method = reqwest::Method::from_bytes(args.method.to_uppercase().as_bytes())
        .map_err(|_| format!("http_request: unknown HTTP method '{}'", args.method))?;

    let mut req = client.request(method, &args.url);

    for (k, v) in &args.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(body) = args.body {
        req = req.body(body);
    }

    let resp = req.send().await
        .map_err(|e| format!("http_request: request failed: {e}"))?;

    let status   = resp.status().as_u16();
    let ok       = resp.status().is_success();

    // Collect response headers (first value per name only).
    let resp_headers: HashMap<String, String> = resp
        .headers()
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|s| (k.to_string(), s.to_string()))
        })
        .collect();

    // Read body up to the cap.
    let bytes = resp.bytes().await
        .map_err(|e| format!("http_request: failed to read response body: {e}"))?;

    let truncated = bytes.len() > MAX_RESPONSE_BYTES;
    let body_slice = &bytes[..bytes.len().min(MAX_RESPONSE_BYTES)];
    let body = String::from_utf8_lossy(body_slice).into_owned();

    let result = HttpResponse { status, ok, headers: resp_headers, body, truncated };
    serde_json::to_string(&result)
        .map_err(|e| format!("http_request: failed to serialize response: {e}"))
}
