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

pub async fn execute_tool(name: &str, arguments: &str) -> Result<String, String> {
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
                        Ok(proxy_json) => invoke_vision_llm(&proxy_json).await,
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
        return execute_web_search(arguments).await;
    }
    // Vision native fallback: runs when vision.wasm is absent (tests, offline).
    if name == "vision" {
        return execute_vision_native(arguments).await;
    }

    // ── 3. Synchronous native fallback ────────────────────────────────────────
    // Used when no `.wasm` binary is present for the tool — ensures full
    // backward compatibility during the Wasm migration.
    match name {
        "calculator"         => execute_calculator(arguments),
        "write_file"         => execute_write_file(arguments),
        "python_interpreter" => python::execute_python(arguments),
        "build_dynamic_ui"   => ui_builder::build_dynamic_ui(arguments),
        "feedback"           => execute_feedback_native(arguments),
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
    matches!(name, "build_dynamic_ui" | "feedback")
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

async fn execute_web_search(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct SearchArgs {
        query: String,
    }

    let args: SearchArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse search arguments: {}", e))?;

    // Use the real Tavily API when a key is present; otherwise fall back to a
    // detailed simulation that gives the LLM enough material to reason with.
    match std::env::var("TAVILY_API_KEY") {
        Ok(key) => tavily_search(&args.query, &key).await,
        Err(_) => Ok(simulated_search(&args.query)),
    }
}

async fn tavily_search(query: &str, api_key: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct TavilyRequest<'a> {
        api_key: &'a str,
        query: &'a str,
        search_depth: &'a str,
        max_results: u8,
    }

    #[derive(serde::Deserialize)]
    struct TavilyResponse {
        results: Vec<TavilyResult>,
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

    Ok(format!("Live search results for '{}':\n\n{}", query, formatted))
}

// ── Vision tool ───────────────────────────────────────────────────────────────

/// Native fallback for the vision tool when `vision.wasm` is not present.
///
/// Reads the requested image from `uploads/`, base64-encodes it, and either:
/// - Calls the gpt-4o vision API when `OPENAI_API_KEY` is available, or
/// - Returns basic file metadata (useful in tests and offline mode).
async fn execute_vision_native(arguments: &str) -> Result<String, String> {
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

    match std::env::var("OPENAI_API_KEY") {
        Ok(key) => {
            let b64 = base64_encode(&bytes);
            call_gpt4v(&b64, mime, &prompt, &key).await
        }
        Err(_) => {
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
async fn invoke_vision_llm(proxy_json: &str) -> Result<String, String> {
    let proxy: VisionProxy = serde_json::from_str(proxy_json)
        .map_err(|e| format!("vision: invalid proxy JSON from Wasm: {}", e))?;

    let prompt = if proxy.prompt.is_empty() {
        "Describe this image in detail. \
         If it contains data (chart, table, graph, text), extract and summarise the key values."
            .to_string()
    } else {
        proxy.prompt.clone()
    };

    match std::env::var("OPENAI_API_KEY") {
        Ok(key) => call_gpt4v(&proxy.image_base64, &proxy.mime_type, &prompt, &key).await,
        Err(_) => Ok(format!(
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
    let q = query.to_lowercase();

    if q.contains("flight") || (q.contains("rome") && (q.contains("fly") || q.contains("air"))) {
        return format!(
            "Search results for '{query}':\n\n\
            1. **Direct Flights to Rome (FCO) — Round Trip from JFK**\n   \
            - Delta Airlines: $289 (direct, 9h 15m) ⭐ Best value\n   \
            - Alitalia / ITA Airways: $312 (direct, 9h 30m)\n   \
            - Lufthansa: $268 (1 stop via Frankfurt, 11h 45m)\n\n\
            2. **Budget Options (indirect)**\n   \
            - Vueling via Barcelona: $198 (12h 30m total)\n   \
            - Ryanair via Madrid: $175 (14h total, carry-on fees apply)\n\n\
            3. **Booking Tips**\n   \
            - Book 6–8 weeks ahead for best prices.\n   \
            - Tuesday/Wednesday departures average 15% cheaper.\n   \
            - Baggage fees not included in budget airline fares.",
        );
    }

    if q.contains("hotel") || (q.contains("rome") && (q.contains("stay") || q.contains("accommodation"))) {
        return format!(
            "Search results for '{query}':\n\n\
            1. **4-Star Hotels — Rome City Centre**\n   \
            - Hotel Nazionale: $145/night (near Pantheon, breakfast included) ⭐ Best value\n   \
            - The Inn at the Spanish Steps: $210/night (boutique luxury)\n   \
            - Bettoja Hotels: $138/night (near Termini, reliable chain)\n\n\
            2. **Mid-Range / Budget**\n   \
            - Residenza Paolo VI: $120/night (Vatican views, 3-star)\n   \
            - Hotel Artorius: $89/night (Trastevere neighbourhood)\n   \
            - Generator Rome (Hostel): $52/night (private room)\n\n\
            3. **Booking Tips**\n   \
            - Most 3–4 star hotels include colazione (Italian breakfast).\n   \
            - Trastevere and Prati districts offer better value than the historic centre.\n   \
            - Book 3+ weeks ahead; prices rise sharply for popular dates.",
        );
    }

    format!(
        "Search results for '{query}':\n\n\
        1. **Destination Overview — Rome, Italy**\n   \
        Rome is a world-class destination with 2,500+ years of history. \
        The city hosts the Colosseum, Vatican Museums, Trevi Fountain, and Piazza Navona.\n\n\
        2. **Best Time to Visit**\n   \
        - Spring (Apr–May): Ideal weather, 18–22°C, moderate crowds.\n   \
        - Autumn (Sep–Oct): Warm, fewer tourists than summer.\n   \
        - Avoid August: Extreme heat, many locals on holiday.\n\n\
        3. **Daily Budget Estimates (per person)**\n   \
        - Budget: $80–$120/day (hostel, street food, free sights)\n   \
        - Mid-range: $150–$250/day (3-star hotel, restaurants, museums)\n   \
        - Luxury: $400+/day\n\n\
        4. **Key Tips**\n   \
        - Roma Pass ($32): Unlimited metro + museum discounts.\n   \
        - Pre-book Vatican and Colosseum tickets (sells out 2+ weeks ahead).\n   \
        - 'Coperto' table cover charge (~€3) is normal at restaurants.",
    )
}
