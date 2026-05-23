mod financial;
mod python;
mod ui_builder;

// ── Vision proxy type (shared between Wasm post-processing and native path) ───

#[derive(serde::Deserialize)]
struct VisionProxy {
    image_base64: String,
    mime_type: String,
    file_path: String,
    #[serde(default)]
    prompt: String,
}

/// Per-request API key overrides.
///
/// These are captured once per HTTP request and threaded through the call chain
/// to avoid the data race caused by `std::env::set_var` in concurrent handlers.
/// Each field falls back to the corresponding environment variable when `None`
/// or empty.
#[derive(Clone, Debug, Default)]
pub struct RequestKeys {
    pub openai_key:        Option<String>,
    pub tavily_key:        Option<String>,
    pub alpha_vantage_key: Option<String>,
}

impl RequestKeys {
    /// Return the effective Tavily key: explicit override → env var → None.
    pub fn tavily(&self) -> Option<String> {
        if let Some(ref k) = self.tavily_key {
            if !k.is_empty() { return Some(k.clone()); }
        }
        std::env::var("TAVILY_API_KEY").ok().filter(|k| !k.is_empty())
    }

    /// Return the effective Alpha Vantage key: explicit override → env var → None.
    pub fn alpha_vantage(&self) -> Option<String> {
        if let Some(ref k) = self.alpha_vantage_key {
            if !k.is_empty() { return Some(k.clone()); }
        }
        std::env::var("ALPHA_VANTAGE_API_KEY").ok().filter(|k| !k.is_empty())
    }
}

pub async fn execute_tool(
    name: &str,
    arguments: &str,
    mission_id: &str,
    keys: &RequestKeys,
) -> Result<String, String> {
    // ── 1. Registry validation + Wasm-first dispatch ──────────────────────────
    //
    // When the registry is initialised (normal production path), we:
    //   a) Reject unknown tool names early with a clear error message.
    //   b) Check for a `.wasm` binary at `professionals/{name}.wasm`.
    //      If the file exists, hand execution to the sandboxed WasmExecutor.
    //
    // The vision tool receives special post-processing: its Wasm binary outputs
    // a base64 proxy JSON rather than a human-readable string.  After Wasm runs
    // we forward the image to a vision-capable LLM and return its description.
    if let Some(reg) = crate::registry::Registry::get() {
        if !reg.contains(name) {
            eprintln!(
                "[WARN] execute_tool: LLM called unregistered tool '{}' — \
                 check the tool name matches the registry exactly. \
                 Arguments were: {}",
                name, arguments
            );
            return Err(format!("Unknown tool: '{}'", name));
        }

        if let Some(wasm_path) = reg.wasm_path_for(name) {
            if wasm_path.exists() {
                // Collect preopens declared by this tool's manifest so the
                // executor can set up the correct WASI filesystem view.
                let preopens = reg
                    .get_tool(name)
                    .map(|t| t.preopens.clone())
                    .unwrap_or_default();

                println!("  🦀 Dispatching '{}' via Wasm sandbox ({:?}).", name, wasm_path);

                // `WasmExecutor::run` is synchronous and wasmtime-wasi internally
                // needs a thread that hasn't already entered a tokio runtime.
                // `spawn_blocking` offloads it to tokio's blocking thread pool.
                let args_owned = arguments.to_owned();
                let wasm_result = tokio::task::spawn_blocking(move || {
                    crate::executor::wasm::WasmExecutor::run(&wasm_path, &args_owned, &preopens)
                })
                .await
                .unwrap_or_else(|e| Err(format!("Wasm executor panicked: {}", e)));

                // Vision post-processing: the Wasm binary outputs a base64 proxy
                // JSON instead of a human-readable description.  We parse it and
                // forward the image to the vision LLM to get the actual analysis.
                if name == "vision" {
                    return match wasm_result {
                        Ok(proxy_json) => invoke_vision_llm(&proxy_json, keys.openai_key.as_deref()).await,
                        Err(e) => Err(e),
                    };
                }

                return wasm_result;
            }
        }
    }

    // ── 2. Async native tools ─────────────────────────────────────────────────
    // Handled before the synchronous dispatch match.
    if name == "web_search" {
        return execute_web_search(arguments, keys.tavily().as_deref()).await;
    }
    // Vision native fallback: runs when vision.wasm is absent (tests, offline).
    if name == "vision" {
        return execute_vision_native(arguments, keys.openai_key.as_deref()).await;
    }
    // Alpha Vantage financial data tools
    if name == "get_company_overview" {
        #[derive(serde::Deserialize)]
        struct Args { symbol: String }
        let args: Args = serde_json::from_str(arguments)
            .map_err(|e| format!("Failed to parse get_company_overview arguments: {}", e))?;
        let av_key = keys.alpha_vantage().ok_or_else(|| {
            "ALPHA_VANTAGE_API_KEY not set".to_string()
        })?;
        return financial::get_company_overview(&args.symbol, &av_key).await;
    }
    if name == "get_price_history" {
        #[derive(serde::Deserialize)]
        struct Args { symbol: String, #[serde(default = "default_period")] period: String }
        fn default_period() -> String { "compact".to_string() }
        let args: Args = serde_json::from_str(arguments)
            .map_err(|e| format!("Failed to parse get_price_history arguments: {}", e))?;
        let av_key = keys.alpha_vantage().ok_or_else(|| {
            "ALPHA_VANTAGE_API_KEY not set".to_string()
        })?;
        return financial::get_price_history(&args.symbol, &args.period, &av_key).await;
    }
    if name == "get_income_statement" {
        #[derive(serde::Deserialize)]
        struct Args { symbol: String }
        let args: Args = serde_json::from_str(arguments)
            .map_err(|e| format!("Failed to parse get_income_statement arguments: {}", e))?;
        let av_key = keys.alpha_vantage().ok_or_else(|| {
            "ALPHA_VANTAGE_API_KEY not set".to_string()
        })?;
        return financial::get_income_statement(&args.symbol, &av_key).await;
    }
    if name == "get_news_sentiment" {
        #[derive(serde::Deserialize)]
        struct Args { symbol: String }
        let args: Args = serde_json::from_str(arguments)
            .map_err(|e| format!("Failed to parse get_news_sentiment arguments: {}", e))?;
        let av_key = keys.alpha_vantage().ok_or_else(|| {
            "ALPHA_VANTAGE_API_KEY not set".to_string()
        })?;
        return financial::get_news_sentiment(&args.symbol, &av_key).await;
    }

    // ── 3. Synchronous native fallback ────────────────────────────────────────
    // Used when no `.wasm` binary is present for the tool — ensures full
    // backward compatibility during the Wasm migration.
    match name {
        "calculator"              => execute_calculator(arguments),
        "write_file"              => execute_write_file(arguments),
        "read_file"               => execute_read_file(arguments),
        "generate_document"       => execute_generate_document(arguments),
        "python_interpreter"      => python::execute_python(arguments),
        "finalize_mission_state"  => ui_builder::finalize_mission_state(arguments),
        "feedback"                => execute_feedback_native(arguments),
        "memory_persist"          => execute_memory_persist(arguments, mission_id),
        _ => {
            eprintln!(
                "[WARN] execute_tool: LLM called unregistered tool '{}' — \
                 check the tool name matches the registry exactly. \
                 Arguments were: {}",
                name, arguments
            );
            Err(format!("Unknown tool: '{}'", name))
        }
    }
}

// ── Memory persist tool ───────────────────────────────────────────────────────

/// Native implementation of the `memory_persist` tool.
///
/// Writes a key/value fact to `memory/global.json` so it can be recalled in
/// any future mission via the `__global_memory` context bus entry.
pub fn execute_memory_persist(arguments: &str, mission_id: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct MemoryPersistArgs {
        key: String,
        value: String,
    }

    let args: MemoryPersistArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse memory_persist arguments: {}", e))?;

    if args.key.trim().is_empty() {
        return Err("memory_persist requires a non-empty 'key'".to_string());
    }

    let store = crate::memory::MemoryStore::new(std::path::Path::new("memory"));
    store.write(&args.key, &args.value, mission_id)?;

    println!(
        "  🧠 memory_persist: stored '{}' for future missions.",
        args.key
    );
    Ok(format!("Remembered '{}' for future missions.", args.key))
}

// ── Feedback tool ─────────────────────────────────────────────────────────────

/// Native implementation of the `feedback` tool.
///
/// Outputs the `AWAITING_FEEDBACK_PREFIX` marker so `validate_mission` can
/// detect that the mission needs a human response before continuing.
fn execute_feedback_native(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct FeedbackArgs {
        question: String,
        #[serde(default)]
        context: String,
    }

    let args: FeedbackArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse feedback arguments: {}", e))?;

    if args.question.trim().is_empty() {
        return Err("Feedback tool requires a non-empty 'question'".to_string());
    }

    // The context summary is appended after a separator so callers can strip
    // the prefix to recover the bare question.
    let marker = if args.context.is_empty() {
        format!(
            "{}{}",
            crate::protocol::AWAITING_FEEDBACK_PREFIX,
            args.question.trim()
        )
    } else {
        format!(
            "{}{}\n\nContext: {}",
            crate::protocol::AWAITING_FEEDBACK_PREFIX,
            args.question.trim(),
            args.context.trim()
        )
    };

    println!(
        "  ⏸️  Feedback tool: mission pausing with question: {}",
        args.question.trim()
    );
    Ok(marker)
}

/// True for tools whose output IS the final task result and needs no follow-up
/// LLM turn.  Avoids the model rephrasing the structured JSON output into prose.
///
/// `feedback` is also terminal: its output is the awaiting-feedback marker
/// which must be passed to `validate_mission` unchanged.
pub fn is_terminal_tool(name: &str) -> bool {
    matches!(name, "finalize_mission_state" | "feedback")
}

fn execute_generate_document(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct TableData {
        headers: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    }

    #[derive(serde::Deserialize)]
    struct Section {
        heading: String,
        content: String,
        #[serde(default)]
        table: Option<TableData>,
    }

    #[derive(serde::Deserialize)]
    struct DocArgs {
        title: String,
        format: String,
        sections: Vec<Section>,
    }

    let args: DocArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse generate_document arguments: {}", e))?;

    if args.sections.is_empty() {
        return Err("generate_document requires at least one section.".to_string());
    }

    let content = match args.format.as_str() {
        "md" => {
            let mut out = format!("# {}\n\n", args.title);
            out += &format!("_Generated by Axion — {}_\n\n", chrono_or_timestamp());
            for s in &args.sections {
                out += &format!("## {}\n\n{}\n\n", s.heading, s.content);
                if let Some(t) = &s.table {
                    out += &format!("| {} |\n", t.headers.join(" | "));
                    out += &format!("| {} |\n", t.headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
                    for row in &t.rows {
                        let cells: Vec<String> = row.iter().map(|v| json_val_str(v)).collect();
                        out += &format!("| {} |\n", cells.join(" | "));
                    }
                    out += "\n";
                }
            }
            out
        }
        "csv" => {
            // CSV: one section per row-block, separated by blank lines.
            let mut out = String::new();
            for s in &args.sections {
                out += &format!("# {}\n", s.heading);
                out += &format!("{}\n", s.content.replace('\n', " "));
                if let Some(t) = &s.table {
                    let esc = |v: &str| {
                        if v.contains(',') || v.contains('"') || v.contains('\n') {
                            format!("\"{}\"", v.replace('"', "\"\""))
                        } else {
                            v.to_string()
                        }
                    };
                    out += &t.headers.iter().map(|h| esc(h)).collect::<Vec<_>>().join(",");
                    out += "\n";
                    for row in &t.rows {
                        let cells: Vec<String> = row.iter().map(|v| esc(&json_val_str(v))).collect();
                        out += &cells.join(",");
                        out += "\n";
                    }
                }
                out += "\n";
            }
            out
        }
        "html" => {
            let mut body = String::new();
            for s in &args.sections {
                body += &format!("<h2>{}</h2>\n<p>{}</p>\n", escape_html(&s.heading), escape_html(&s.content).replace('\n', "<br>"));
                if let Some(t) = &s.table {
                    body += "<table>\n<thead><tr>";
                    for h in &t.headers {
                        body += &format!("<th>{}</th>", escape_html(h));
                    }
                    body += "</tr></thead>\n<tbody>\n";
                    for row in &t.rows {
                        body += "<tr>";
                        for v in row {
                            body += &format!("<td>{}</td>", escape_html(&json_val_str(v)));
                        }
                        body += "</tr>\n";
                    }
                    body += "</tbody></table>\n";
                }
            }
            format!(
                r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
  body {{font-family:-apple-system,BlinkMacSystemFont,"Inter",sans-serif;background:#07090c;color:#e5e7eb;max-width:900px;margin:40px auto;padding:0 24px;}}
  h1 {{font-size:2rem;font-weight:900;margin-bottom:8px;color:#fff;}}
  h2 {{font-size:1.2rem;font-weight:700;margin-top:36px;margin-bottom:8px;color:rgba(255,255,255,0.85);}}
  p  {{color:rgba(255,255,255,0.60);line-height:1.7;}}
  table {{width:100%;border-collapse:collapse;margin-top:12px;}}
  th  {{text-align:left;padding:8px 12px;background:rgba(255,255,255,0.06);color:rgba(255,255,255,0.80);font-size:12px;text-transform:uppercase;letter-spacing:.06em;border-bottom:1px solid rgba(255,255,255,0.10);}}
  td  {{padding:8px 12px;color:rgba(255,255,255,0.60);border-bottom:1px solid rgba(255,255,255,0.06);font-size:14px;}}
  .meta {{font-size:12px;color:rgba(255,255,255,0.28);margin-bottom:32px;}}
</style>
</head><body>
<h1>{title}</h1>
<p class="meta">Generated by Axion</p>
{body}
</body></html>"#,
                title = escape_html(&args.title),
                body = body,
            )
        }
        _ => return Err(format!("Unknown format '{}'. Use md, csv, or html.", args.format)),
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let slug: String = args
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect::<String>();
    let slug = &slug[..slug.len().min(32)];

    let filename = format!("doc_{}_{}.{}", timestamp, slug, args.format);
    let missions_dir = std::path::Path::new("missions");
    std::fs::create_dir_all(missions_dir)
        .map_err(|e| format!("Failed to create missions/: {}", e))?;

    let path = missions_dir.join(&filename);
    std::fs::write(&path, &content)
        .map_err(|e| format!("Failed to write '{}': {}", filename, e))?;

    println!("  📄 generate_document: wrote '{}'", filename);
    Ok(format!("Document '{}' saved ({} bytes). Ready to download.", filename, content.len()))
}

fn json_val_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null      => String::new(),
        other                        => other.to_string(),
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn chrono_or_timestamp() -> String {
    // Simple ISO-style date without the chrono crate.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // epoch seconds → YYYY-MM-DD approximation (good enough for a report header)
    let days   = ts / 86400;
    let years  = 1970 + days / 365;
    let month  = (days % 365) / 30 + 1;
    let day    = (days % 365) % 30 + 1;
    format!("{}-{:02}-{:02}", years, month, day)
}

fn execute_calculator(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct CalcArgs {
        operation: String,
        #[serde(default)]
        values: Vec<f64>,
    }

    let args: CalcArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse calculator arguments: {}", e))?;

    if args.values.is_empty() {
        return Err("No values provided for calculation".to_string());
    }

    let result = match args.operation.as_str() {
        "add" => args.values.iter().sum::<f64>(),
        "subtract" => args.values[1..].iter().fold(args.values[0], |acc, &x| acc - x),
        "multiply" => args.values.iter().fold(1.0, |acc, &x| acc * x),
        "divide" => {
            let mut result = args.values[0];
            for &val in &args.values[1..] {
                if val == 0.0 {
                    return Err("Division by zero".to_string());
                }
                result /= val;
            }
            result
        }
        _ => return Err(format!("Unknown operation: {}", args.operation)),
    };

    Ok(format!("Result: {}", result))
}

fn execute_write_file(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct WriteArgs {
        filename: String,
        content: String,
    }

    let args: WriteArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse write_file arguments: {}", e))?;

    // Derive a safe slug from the provided filename: strip extension and any
    // unsafe characters, then compose a timestamped path inside missions/.
    let slug: String = args
        .filename
        .trim_end_matches(".md")
        .trim_end_matches(".txt")
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' })
        .collect::<String>()
        .to_lowercase();

    if slug.is_empty() || slug.contains("..") {
        return Err(format!("Invalid filename '{}'", args.filename));
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let final_name = format!("mission_{}_{}.md", timestamp, slug);
    let missions_dir = std::path::Path::new("missions");
    std::fs::create_dir_all(missions_dir)
        .map_err(|e| format!("Failed to create missions/ directory: {}", e))?;

    let path = missions_dir.join(&final_name);
    std::fs::write(&path, &args.content)
        .map_err(|e| format!("Failed to write '{}': {}", final_name, e))?;

    Ok(format!("File 'missions/{}' written successfully.", final_name))
}

fn execute_read_file(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct ReadArgs {
        filename: String,
    }

    let args: ReadArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse read_file arguments: {}", e))?;

    let name = args.filename.trim();

    // Security: reject empty names, path traversal, and directory separators.
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
    {
        return Err(format!("Invalid filename: '{}'", name));
    }

    // Only allow the extensions we accept at the upload endpoint.
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !matches!(ext.as_str(), "csv" | "json" | "txt") {
        return Err(format!(
            "read_file only supports CSV, JSON, and TXT files (got '.{}').",
            ext
        ));
    }

    let path = std::path::Path::new("uploads").join(name);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read '{}': {}", name, e))?;

    // Cap output to avoid flooding the context window.
    const MAX_BYTES: usize = 32_768;
    if content.len() > MAX_BYTES {
        let truncated = &content[..MAX_BYTES];
        println!("  📂 read_file: '{}' ({} bytes, truncated to {})", name, content.len(), MAX_BYTES);
        return Ok(format!(
            "File: {}\nSize: {} bytes (showing first {} bytes)\n\n{}",
            name,
            content.len(),
            MAX_BYTES,
            truncated
        ));
    }

    println!("  📂 read_file: '{}' ({} bytes)", name, content.len());
    Ok(format!("File: {}\nSize: {} bytes\n\n{}", name, content.len(), content))
}

/// `tavily_key_override` is the per-request Tavily key (already resolved via
/// `RequestKeys::tavily()`); pass `None` to use only the env var.
async fn execute_web_search(arguments: &str, tavily_key_override: Option<&str>) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct SearchArgs {
        query: String,
    }

    let args: SearchArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse search arguments: {}", e))?;

    // Use the per-request key when provided; otherwise fall back to the env var.
    // Never call set_var — read directly to avoid the concurrent-handler race.
    let effective_key = tavily_key_override
        .map(|k| k.to_owned())
        .or_else(|| std::env::var("TAVILY_API_KEY").ok().filter(|k| !k.is_empty()));

    match effective_key {
        Some(key) => tavily_search(&args.query, &key).await,
        None => Ok(simulated_search(&args.query)),
    }
}

async fn tavily_search(query: &str, api_key: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct TavilyRequest<'a> {
        api_key: &'a str,
        query: &'a str,
        search_depth: &'a str,
        max_results: u8,
        include_images: bool,
    }

    #[derive(serde::Deserialize)]
    struct TavilyResponse {
        results: Vec<TavilyResult>,
        #[serde(default)]
        images: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct TavilyResult {
        title: String,
        url: String,
        content: String,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&TavilyRequest {
            api_key,
            query,
            search_depth: "basic",
            max_results: 5,
            include_images: true,
        })
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Tavily API error {}: {}", status, body));
    }

    let body: TavilyResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {}", e))?;

    if body.results.is_empty() {
        return Ok(format!("No results found for '{}'.", query));
    }

    let formatted = body
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}\n   {}\n   Source: {}", i + 1, r.title, r.content, r.url))
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut output = format!("Live search results for '{}':\n\n{}", query, formatted);

    if !body.images.is_empty() {
        let image_lines = body.images
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, url)| format!("  Image {}: {}", i + 1, url))
            .collect::<Vec<_>>()
            .join("\n");
        output.push_str(&format!("\n\nVerified images for this topic:\n{}", image_lines));
    }

    Ok(output)
}

// ── Vision tool ───────────────────────────────────────────────────────────────

/// Native fallback for the vision tool when `vision.wasm` is not present.
///
/// Reads the requested image from `uploads/`, base64-encodes it, and either:
/// - Calls the gpt-4o vision API when an OpenAI key is available, or
/// - Returns basic file metadata (useful in tests and offline mode).
///
/// `openai_key_override` is the per-request OpenAI key; `None` falls back to
/// `OPENAI_API_KEY` env var.
async fn execute_vision_native(arguments: &str, openai_key_override: Option<&str>) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct VisionArgs {
        file_path: String,
        #[serde(default)]
        prompt: Option<String>,
    }

    let args: VisionArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse vision arguments: {}", e))?;

    // Sanitize: reject path traversal.
    let name = args.file_path.trim_start_matches('/');
    if name.contains("..") || name.contains('\\') || name.is_empty() {
        return Err(format!("Invalid file_path: '{}'", args.file_path));
    }

    let uploads = std::path::Path::new("uploads");
    std::fs::create_dir_all(uploads)
        .map_err(|e| format!("Cannot create uploads/ directory: {}", e))?;

    let path = uploads.join(name);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Cannot read image '{}': {}", path.display(), e))?;

    let mime = mime_from_ext(path.extension().and_then(|e| e.to_str()).unwrap_or(""));
    let prompt = args.prompt.unwrap_or_else(|| {
        "Describe this image in detail. \
         If it contains data (chart, table, graph, text), extract and summarise the key values."
            .to_string()
    });

    // Per-request key takes priority; fall back to env var.
    let effective_key = openai_key_override
        .map(|k| k.to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty()));

    match effective_key {
        Some(key) => {
            let b64 = base64_encode(&bytes);
            call_gpt4v(&b64, mime, &prompt, &key).await
        }
        None => {
            // No API key — return file metadata so tests and offline runs work.
            Ok(format!(
                "Image file '{}' loaded ({}, {} bytes). \
                 [Set OPENAI_API_KEY to enable AI vision analysis]",
                name,
                mime,
                bytes.len()
            ))
        }
    }
}

/// Called after `vision.wasm` produces a base64 proxy JSON.
/// Extracts the image data and forwards it to the gpt-4o vision API.
///
/// `openai_key_override` is the per-request OpenAI key; `None` falls back to
/// `OPENAI_API_KEY` env var.
async fn invoke_vision_llm(proxy_json: &str, openai_key_override: Option<&str>) -> Result<String, String> {
    let proxy: VisionProxy = serde_json::from_str(proxy_json)
        .map_err(|e| format!("vision: invalid proxy JSON from Wasm: {}", e))?;

    let prompt = if proxy.prompt.is_empty() {
        "Describe this image in detail. \
         If it contains data (chart, table, graph, text), extract and summarise the key values."
            .to_string()
    } else {
        proxy.prompt.clone()
    };

    let effective_key = openai_key_override
        .map(|k| k.to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.is_empty()));

    match effective_key {
        Some(key) => call_gpt4v(&proxy.image_base64, &proxy.mime_type, &prompt, &key).await,
        None => Ok(format!(
            "Image '{}' ({}) prepared for vision analysis ({} base64 chars). \
             [Set OPENAI_API_KEY to enable AI analysis]",
            proxy.file_path,
            proxy.mime_type,
            proxy.image_base64.len()
        )),
    }
}

/// Send a base64-encoded image to gpt-4o and return the vision description.
async fn call_gpt4v(
    image_b64: &str,
    mime_type: &str,
    prompt: &str,
    api_key: &str,
) -> Result<String, String> {
    use serde_json::json;

    let data_url = format!("data:{};base64,{}", mime_type, image_b64);

    let body = json!({
        "model": "gpt-4o",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }],
        "max_tokens": 1024,
        "temperature": 0.0
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Vision API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Vision API error {}: {}", status, text));
    }

    #[derive(serde::Deserialize)]
    struct Resp { choices: Vec<Choice> }
    #[derive(serde::Deserialize)]
    struct Choice { message: Msg }
    #[derive(serde::Deserialize)]
    struct Msg { content: Option<String> }

    let parsed: Resp = resp.json().await
        .map_err(|e| format!("Failed to parse vision API response: {}", e))?;

    parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Empty vision API response".to_string())
}

fn mime_from_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "png"        => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif"        => "image/gif",
        "webp"       => "image/webp",
        "bmp"        => "image/bmp",
        _            => "application/octet-stream",
    }
}

/// RFC 4648 standard base64 encoder (avoids adding an extra crate dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn simulated_search(query: &str) -> String {
    format!(
        "Web search unavailable (no TAVILY_API_KEY configured).\n\
         Query: \"{query}\"\n\n\
         Proceed using training knowledge. Note in your response that these findings \
         are based on pre-training data and may not reflect current information. \
         Do not fabricate specific statistics, prices, or recent events."
    )
}
