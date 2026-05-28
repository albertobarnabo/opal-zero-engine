//! Research Professional manifests.
//!
//! TOML files in `professionals/research/` declare domain-specific research
//! strategies.  When the Dispatcher routes a WebSearcher task whose intent
//! matches a manifest's `match_patterns`, the kernel:
//!
//!  1. **Extracts variables** from the task intent — regex-first, single LLM
//!     call as fallback.
//!  2. **Collects sources deterministically** — calls Rust functions directly,
//!     no LLM tool calls.  Required sources abort on failure; optional sources
//!     contribute an "unavailable" note so the LLM knows data is missing.
//!  3. **Runs one extraction-only LLM pass** — the LLM receives all raw source
//!     data and produces structured research findings.  No further tool calls.
//!
//! ## Source types
//!
//! Each `[[sources]]` entry has a `type` field (default `"native"`):
//!
//! | `type`    | Description |
//! |-----------|-------------|
//! | `native`  | OpalZero built-in tool (`get_company_overview`, etc.) |
//! | `mcp`     | External MCP server — HTTP or stdio subprocess |
//!
//! MCP sources add three fields:
//! - `server` — `https://host/path` or `stdio:command [args]`
//! - `tool_name` — the tool to call on the MCP server
//! - `api_key_env` — env-var name holding the API key (HTTP only)
//!
//! Param values (both native and MCP) may contain `{{var_name}}` placeholders
//! that are substituted with variables extracted from the task intent.
//!
//! If no manifest matches the intent, or if the manifest path errors, the
//! Dispatcher falls through to the normal Exa multi-turn search.

pub mod mcp_client;

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use crate::engine::{AiProvider, ToolResponse};
use crate::tools::RequestKeys;

// ── Global registry ───────────────────────────────────────────────────────────

static REGISTRY: OnceLock<Vec<ResearchManifest>> = OnceLock::new();

// ── Manifest structs ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ResearchManifest {
    pub manifest:  ManifestMeta,
    #[serde(default)]
    pub variables: HashMap<String, VarSpec>,
    #[serde(default)]
    pub sources:   Vec<SourceSpec>,
}

#[derive(Debug, Deserialize)]
pub struct ManifestMeta {
    pub id:          String,
    pub description: String,
    #[serde(default)]
    pub match_patterns: Vec<String>,
}

/// Template variable that must be extracted from the task intent before sources
/// can be fetched (e.g. a stock ticker symbol).
#[derive(Debug, Deserialize)]
pub struct VarSpec {
    /// Optional regex with exactly one capture group that extracts the value
    /// directly from the intent string.  If absent or if the regex doesn't
    /// match, the LLM extraction fallback is used.
    #[serde(default)]
    pub regex: Option<String>,
    /// Natural-language hint for the LLM fallback prompt.
    pub hint: String,
    /// Whether the manifest path should be aborted when this variable cannot
    /// be extracted (falls through to Exa search on abort).
    #[serde(default)]
    pub required: bool,
}

/// Selects the execution backend for a `[[sources]]` entry.
#[derive(Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// Built-in OpalZero tool (Alpha Vantage financial data, etc.).
    #[default]
    Native,
    /// External MCP server — HTTP JSON-RPC or stdio subprocess.
    Mcp,
}

/// A single data source within a research manifest.
#[derive(Debug, Deserialize)]
pub struct SourceSpec {
    pub id: String,

    /// Source backend: `"native"` (default) or `"mcp"`.
    #[serde(rename = "type", default)]
    pub kind: SourceKind,

    // ── Native source ─────────────────────────────────────────────────────────
    /// OpalZero tool name for `kind = native` (e.g. `"get_company_overview"`).
    #[serde(default)]
    pub tool: String,

    // ── MCP source ────────────────────────────────────────────────────────────
    /// MCP server endpoint for `kind = mcp`.
    /// Use `"https://host/path"` for HTTP transport or
    /// `"stdio:command [args]"` to spawn a subprocess.
    #[serde(default)]
    pub server: Option<String>,

    /// Tool name to call on the MCP server.
    #[serde(default)]
    pub tool_name: Option<String>,

    /// Name of the environment variable holding the API key.
    /// Sent as `Authorization: Bearer …` for HTTP transport; ignored for stdio.
    #[serde(default)]
    pub api_key_env: Option<String>,

    // ── Shared ────────────────────────────────────────────────────────────────
    #[serde(default)]
    pub description: String,

    /// Arguments forwarded to the tool.  Values may contain `{{var_name}}`
    /// placeholders substituted with variables extracted from the task intent.
    #[serde(default)]
    pub params: HashMap<String, String>,

    /// Abort the manifest path when `true` and this source errors.
    /// Optional sources contribute an "[unavailable]" note instead.
    #[serde(default)]
    pub required: bool,
}

// ── Registry initialisation ───────────────────────────────────────────────────

/// Load research manifests from `dir` into the global registry.
/// Safe to call multiple times — subsequent calls are no-ops.
pub fn init(dir: &Path) {
    if REGISTRY.get().is_some() {
        return;
    }
    let manifests = load_from_dir(dir);
    tracing::info!(count = manifests.len(), "ResearchRegistry: manifests loaded");
    let _ = REGISTRY.set(manifests);
}

/// Try several path strategies to find `professionals/research/` and
/// initialise the registry.  Called once at server startup.
pub fn init_default() {
    // Strategy 1: compile-time path (works for opalzero-core and opalzero-server)
    let compile_time = concat!(env!("CARGO_MANIFEST_DIR"), "/professionals/research");
    if Path::new(compile_time).is_dir() {
        return init(Path::new(compile_time));
    }

    // Strategy 2: workspace root relative (opalzero-server → opalzero-core)
    let ws_path = std::path::PathBuf::from(compile_time)
        .parent()       // strip "/professionals/research"
        .and_then(|p| p.parent())  // strip crate dir → workspace root
        .map(|p| p.join("opalzero-core/professionals/research"));
    if let Some(path) = ws_path {
        if path.is_dir() {
            return init(&path);
        }
    }

    // Strategy 3: CWD-relative fallback
    init(Path::new("professionals/research"));
}

fn load_from_dir(dir: &Path) -> Vec<ResearchManifest> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        tracing::warn!(dir = ?dir, "ResearchRegistry: directory not found — no manifests loaded");
        return vec![];
    };

    let mut manifests = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<ResearchManifest>(&content) {
                Ok(m) => {
                    tracing::debug!(id = %m.manifest.id, "ResearchRegistry: loaded");
                    manifests.push(m);
                }
                Err(e) => {
                    tracing::warn!(path = ?path, error = %e, "ResearchRegistry: parse error — skipping");
                }
            },
            Err(e) => {
                tracing::warn!(path = ?path, error = %e, "ResearchRegistry: read error — skipping");
            }
        }
    }
    manifests
}

// ── Public interface ──────────────────────────────────────────────────────────

/// Return the first manifest whose `match_patterns` appear (case-insensitive
/// substring match) in `intent`, or `None` if no manifest is loaded / matches.
pub fn try_match(intent: &str) -> Option<&'static ResearchManifest> {
    let lower = intent.to_lowercase();
    REGISTRY.get()?.iter().find(|m| {
        m.manifest
            .match_patterns
            .iter()
            .any(|pat| lower.contains(&pat.to_lowercase()))
    })
}

/// Collect all source data deterministically, then run one extraction-only LLM
/// pass.  Returns the structured findings string that the Dispatcher stores as
/// `task.result`.
///
/// Errors propagate to the Dispatcher, which falls through to the Exa path.
pub async fn run_manifest(
    manifest:      &ResearchManifest,
    intent:        &str,
    output_schema: Option<&str>,
    keys:          &RequestKeys,
    provider:      &dyn AiProvider,
) -> Result<String, String> {
    // ── 1. Extract template variables ─────────────────────────────────────────
    let vars = extract_vars(manifest, intent, provider).await;

    // Abort on missing required variables
    for (name, spec) in &manifest.variables {
        if spec.required && vars.get(name).map(|v| v.is_empty()).unwrap_or(true) {
            return Err(format!(
                "Required variable '{}' could not be extracted from intent",
                name
            ));
        }
    }

    tracing::info!(
        manifest = %manifest.manifest.id,
        vars     = ?vars,
        "Research manifest: variables resolved"
    );

    // ── 2. Collect sources ────────────────────────────────────────────────────
    let mut blocks = Vec::new();
    for source in &manifest.sources {
        match execute_source(source, &vars, keys).await {
            Ok(data) => {
                tracing::debug!(source = %source.id, "Research manifest: source collected");
                let header = if source.description.is_empty() {
                    source.id.replace('_', " ").to_uppercase()
                } else {
                    format!(
                        "{} — {}",
                        source.id.replace('_', " ").to_uppercase(),
                        source.description
                    )
                };
                blocks.push(format!("### {header}\n{data}"));
            }
            Err(e) if source.required => {
                return Err(format!("Required source '{}' failed: {}", source.id, e));
            }
            Err(e) => {
                tracing::warn!(
                    source = %source.id,
                    error  = %e,
                    "Research manifest: optional source unavailable"
                );
                blocks.push(format!(
                    "### {}\n[unavailable: {}]",
                    source.id.replace('_', " ").to_uppercase(),
                    e
                ));
            }
        }
    }

    let raw_data = blocks.join("\n\n");

    // ── 3. Build extraction-only prompt ───────────────────────────────────────
    // Include output_schema keys so the LLM knows exactly which facts to surface.
    let schema_hint = output_schema
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
        .filter(|obj| !obj.is_empty())
        .map(|obj| {
            let keys: String = obj.keys().map(|k| format!("  - {k}\n")).collect();
            format!(
                "\n\nREQUIRED DATA POINTS — include specific values for ALL of these:\n{keys}"
            )
        })
        .unwrap_or_default();

    let extraction_prompt = format!(
        "You are a research analyst. The following data was collected from authoritative \
         sources for this task:\n\n\
         TASK: {intent}{schema_hint}\n\n\
         SOURCE DATA:\n{raw_data}\n\n\
         INSTRUCTIONS:\n\
         - Write a clear, structured research summary using ONLY the data above.\n\
         - Include all specific numbers (prices, ratios, revenues, percentages).\n\
         - Do NOT search the web or call any tools — the data collection is complete.\n\
         - If any section is marked [unavailable], note it briefly.\n\
         - Structure your findings so an Analyst can extract precise values easily.\n\n\
         Research summary:",
    );

    // ── 4. Single extraction-only LLM pass ───────────────────────────────────
    match provider.generate_response(&extraction_prompt, None).await {
        Ok(ToolResponse::Text(text)) => Ok(text),
        Ok(ToolResponse::ToolCall { name, .. }) => Err(format!(
            "LLM unexpectedly called tool '{name}' during extraction-only pass"
        )),
        Err(e) => Err(format!("LLM extraction pass failed: {e}")),
    }
}

// ── Variable extraction ───────────────────────────────────────────────────────

async fn extract_vars(
    manifest: &ResearchManifest,
    intent:   &str,
    provider: &dyn AiProvider,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();

    for (name, spec) in &manifest.variables {
        // Regex first — try capture groups 1..N, use the first non-empty match
        if let Some(ref pattern) = spec.regex {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(cap) = re.captures(intent) {
                    // Walk capture groups; group 0 is the full match, skip it
                    let extracted = (1..=cap.len().saturating_sub(1))
                        .find_map(|i| cap.get(i))
                        .map(|m| m.as_str().to_string());
                    if let Some(value) = extracted {
                        if !value.is_empty() {
                            tracing::debug!(var = name, value, "extracted via regex");
                            vars.insert(name.clone(), value);
                            continue;
                        }
                    }
                }
            }
        }

        // LLM fallback — one targeted call, no tools
        let prompt = format!(
            "Extract {name} from the following request. Reply with ONLY the extracted \
             value — no explanation, no punctuation, just the bare value.\n\
             Hint: {hint}\n\
             Request: \"{intent}\"",
            name  = name,
            hint  = spec.hint,
        );
        if let Ok(ToolResponse::Text(val)) = provider.generate_response(&prompt, None).await {
            let trimmed = val.trim().to_string();
            if !trimmed.is_empty() {
                tracing::debug!(var = name, value = trimmed, "extracted via LLM fallback");
                vars.insert(name.clone(), trimmed);
            }
        }
    }

    vars
}

// ── Template variable substitution ───────────────────────────────────────────

/// Replace every `{{key}}` placeholder in `template` with the corresponding
/// value from `vars`.  Unrecognised placeholders are left unchanged so the
/// caller can surface a clear error rather than silently passing a broken value.
fn substitute_vars(template: &str, vars: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{}}}}}", key), value);
    }
    result
}

/// Convert a `params` map (after template substitution) to a `serde_json::Value`
/// object.  Numeric-looking strings become JSON numbers; `"true"` / `"false"`
/// become JSON booleans; everything else stays a string.
fn params_to_json(
    params: &HashMap<String, String>,
    vars:   &HashMap<String, String>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (key, template) in params {
        let val = substitute_vars(template, vars);
        let json_val = if let Ok(n) = val.parse::<i64>() {
            serde_json::Value::Number(n.into())
        } else if let Ok(f) = val.parse::<f64>() {
            serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::String(val.clone()))
        } else if val == "true" {
            serde_json::Value::Bool(true)
        } else if val == "false" {
            serde_json::Value::Bool(false)
        } else {
            serde_json::Value::String(val)
        };
        obj.insert(key.clone(), json_val);
    }
    serde_json::Value::Object(obj)
}

// ── Source execution ──────────────────────────────────────────────────────────

async fn execute_source(
    source: &SourceSpec,
    vars:   &HashMap<String, String>,
    keys:   &RequestKeys,
) -> Result<String, String> {
    match source.kind {
        SourceKind::Native => execute_native_source(source, vars, keys).await,
        SourceKind::Mcp    => execute_mcp_source(source, vars).await,
    }
}

// ── Native source execution ───────────────────────────────────────────────────
//
// Routes to OpalZero built-in tools via `execute_tool`, keeping key resolution
// and registry validation consistent with the normal agent path.
//
// Alpha Vantage tools receive special argument construction because their
// primary input (the ticker symbol) comes from the extracted variables, not
// from explicit manifest params.  Every other registered tool (web_search,
// fetch_page, rss_reader, …) receives its arguments directly from the manifest
// params after template substitution.

async fn execute_native_source(
    source: &SourceSpec,
    vars:   &HashMap<String, String>,
    keys:   &RequestKeys,
) -> Result<String, String> {
    let arguments = match source.tool.as_str() {
        // Alpha Vantage tools: inject the extracted ticker as "symbol"
        "get_company_overview" | "get_income_statement" | "get_news_sentiment" => {
            let ticker = vars.get("ticker").map(|s| s.as_str()).unwrap_or("");
            serde_json::json!({ "symbol": ticker }).to_string()
        }
        "get_price_history" => {
            let ticker = vars.get("ticker").map(|s| s.as_str()).unwrap_or("");
            let period = source.params.get("period").map(|s| s.as_str()).unwrap_or("compact");
            serde_json::json!({ "symbol": ticker, "period": period }).to_string()
        }
        // General tools (web_search, fetch_page, rss_reader, …): convert
        // params to JSON with template substitution.  Any registered OpalZero
        // tool works here as long as its expected arguments match the params.
        _ => params_to_json(&source.params, vars).to_string(),
    };

    crate::tools::execute_tool(&source.tool, &arguments, "research-manifest", keys).await
}

// ── MCP source execution ──────────────────────────────────────────────────────

async fn execute_mcp_source(
    source: &SourceSpec,
    vars:   &HashMap<String, String>,
) -> Result<String, String> {
    let server = source
        .server
        .as_deref()
        .ok_or("MCP source is missing 'server' field")?;
    let tool_name = source
        .tool_name
        .as_deref()
        .ok_or("MCP source is missing 'tool_name' field")?;

    // Resolve API key from the named environment variable (if declared).
    let api_key_owned: Option<String> = source
        .api_key_env
        .as_deref()
        .and_then(|env_var| std::env::var(env_var).ok())
        .filter(|k| !k.is_empty());
    let api_key = api_key_owned.as_deref();

    // Substitute template vars into every param value, then convert to JSON.
    let arguments = params_to_json(&source.params, vars);

    tracing::debug!(
        source = %source.id,
        server,
        tool_name,
        "MCP source: calling tool"
    );

    mcp_client::call_mcp_tool(server, tool_name, arguments, api_key).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a minimal native manifest in memory (no disk I/O).
    fn equity_manifest() -> ResearchManifest {
        let toml_str = r#"
[manifest]
id = "equity_research"
description = "test"
match_patterns = ["stock", "equity", "ticker"]

[variables.ticker]
regex    = "\\$([A-Z]{1,5})|\\b([A-Z]{2,5})\\b"
hint     = "stock ticker symbol"
required = true

[[sources]]
id       = "company_overview"
tool     = "get_company_overview"
required = true

[[sources]]
id       = "price_history"
tool     = "get_price_history"
required = true
params   = { period = "compact" }
"#;
        toml::from_str(toml_str).expect("test manifest must parse")
    }

    // Helper: build a manifest with an MCP source.
    fn mcp_manifest() -> ResearchManifest {
        let toml_str = r#"
[manifest]
id = "mcp_test"
description = "test mcp"
match_patterns = ["news", "headlines"]

[variables.company]
hint     = "company name"
required = true

[[sources]]
id          = "exa_news"
type        = "mcp"
server      = "https://mcp.example.com"
tool_name   = "web_search"
api_key_env = "TEST_API_KEY"
required    = false
params      = { query = "{{company}} latest news", numResults = "5" }
"#;
        toml::from_str(toml_str).expect("mcp test manifest must parse")
    }

    #[test]
    fn native_manifest_deserializes() {
        let m = equity_manifest();
        assert_eq!(m.manifest.id, "equity_research");
        assert_eq!(m.sources.len(), 2);
        assert!(m.variables.contains_key("ticker"));
        assert_eq!(m.sources[0].kind, SourceKind::Native);
    }

    #[test]
    fn mcp_manifest_deserializes() {
        let m = mcp_manifest();
        assert_eq!(m.sources.len(), 1);
        let src = &m.sources[0];
        assert_eq!(src.kind, SourceKind::Mcp);
        assert_eq!(src.server.as_deref(), Some("https://mcp.example.com"));
        assert_eq!(src.tool_name.as_deref(), Some("web_search"));
        assert_eq!(src.api_key_env.as_deref(), Some("TEST_API_KEY"));
        assert_eq!(src.params.get("numResults").map(|s| s.as_str()), Some("5"));
    }

    #[test]
    fn substitute_vars_replaces_placeholders() {
        let mut vars = HashMap::new();
        vars.insert("ticker".to_string(), "AAPL".to_string());
        vars.insert("year".to_string(), "2024".to_string());

        assert_eq!(
            substitute_vars("{{ticker}} earnings {{year}}", &vars),
            "AAPL earnings 2024"
        );
        // Unrecognised placeholder stays unchanged
        assert_eq!(
            substitute_vars("{{ticker}} {{unknown}}", &vars),
            "AAPL {{unknown}}"
        );
    }

    #[test]
    fn params_to_json_converts_types() {
        let mut params = HashMap::new();
        params.insert("query".to_string(), "{{ticker}} news".to_string());
        params.insert("numResults".to_string(), "5".to_string());
        params.insert("useAutoprompt".to_string(), "true".to_string());

        let mut vars = HashMap::new();
        vars.insert("ticker".to_string(), "TSLA".to_string());

        let json = params_to_json(&params, &vars);
        assert_eq!(json["query"].as_str(), Some("TSLA news"));
        assert_eq!(json["numResults"].as_i64(), Some(5));
        assert_eq!(json["useAutoprompt"].as_bool(), Some(true));
    }

    #[test]
    fn regex_extracts_dollar_prefixed_ticker() {
        let re = regex::Regex::new(r"\$([A-Z]{1,5})|\b([A-Z]{2,5})\b").unwrap();
        let cap = re.captures("Tell me about $AAPL stock").unwrap();
        let value = (1..=cap.len().saturating_sub(1))
            .find_map(|i| cap.get(i))
            .map(|m| m.as_str().to_string())
            .unwrap();
        assert_eq!(value, "AAPL");
    }

    #[test]
    fn regex_extracts_bare_ticker() {
        let re = regex::Regex::new(r"\$([A-Z]{1,5})|\b([A-Z]{2,5})\b").unwrap();
        let cap = re.captures("Research NVDA earnings").unwrap();
        let value = (1..=cap.len().saturating_sub(1))
            .find_map(|i| cap.get(i))
            .map(|m| m.as_str().to_string())
            .unwrap();
        assert_eq!(value, "NVDA");
    }

    #[test]
    fn manifest_match_patterns_case_insensitive() {
        let m = equity_manifest();
        let lower = "analyze this stock for me".to_lowercase();
        assert!(
            m.manifest.match_patterns.iter().any(|p| lower.contains(&p.to_lowercase())),
            "should match 'stock'"
        );

        let no_match = "write a poem about the moon".to_lowercase();
        assert!(
            !m.manifest.match_patterns.iter().any(|p| no_match.contains(&p.to_lowercase())),
            "should not match"
        );
    }

    // ── New manifest TOML files parse correctly ───────────────────────────────

    fn load_toml_manifest(filename: &str) -> ResearchManifest {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("professionals/research")
            .join(filename);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {filename}: {e}"));
        toml::from_str(&content)
            .unwrap_or_else(|e| panic!("{filename} TOML parse error: {e}"))
    }

    #[test]
    fn equity_toml_file_parses() {
        let m = load_toml_manifest("equity.toml");
        assert_eq!(m.manifest.id, "equity_research");
        assert!(m.sources.iter().all(|s| s.kind == SourceKind::Native));
        assert!(m.variables.contains_key("ticker"));
    }

    #[test]
    fn company_intelligence_toml_parses() {
        let m = load_toml_manifest("company_intelligence.toml");
        assert_eq!(m.manifest.id, "company_intelligence");
        assert!(m.variables.contains_key("company"));
        assert!(m.sources.iter().all(|s| s.tool == "web_search"));
        assert!(m.sources.iter().all(|s| s.kind == SourceKind::Native));
        // Every web_search source must declare a query param
        for src in &m.sources {
            assert!(src.params.contains_key("query"), "{}: missing query param", src.id);
        }
    }

    #[test]
    fn founder_research_toml_parses() {
        let m = load_toml_manifest("founder_research.toml");
        assert_eq!(m.manifest.id, "founder_research");
        assert!(m.variables.contains_key("person"));
        assert!(m.sources.iter().all(|s| s.tool == "web_search"));
        for src in &m.sources {
            // Query templates must reference {{person}}
            let query = src.params.get("query").expect("missing query");
            assert!(
                query.contains("{{person}}"),
                "{}: query '{}' must contain {{{{person}}}}",
                src.id, query
            );
        }
    }

    #[test]
    fn competitive_landscape_toml_parses() {
        let m = load_toml_manifest("competitive_landscape.toml");
        assert_eq!(m.manifest.id, "competitive_landscape");
        assert!(m.variables.contains_key("company"));
        for src in &m.sources {
            let query = src.params.get("query").expect("missing query");
            assert!(
                query.contains("{{company}}"),
                "{}: query '{}' must contain {{{{company}}}}",
                src.id, query
            );
        }
    }

    #[test]
    fn company_news_toml_parses() {
        let m = load_toml_manifest("company_news.toml");
        assert_eq!(m.manifest.id, "company_news");
        // Has at least one MCP source
        assert!(
            m.sources.iter().any(|s| s.kind == SourceKind::Mcp),
            "company_news should have at least one MCP source"
        );
    }

    // ── General tool path for web_search ──────────────────────────────────────

    #[test]
    fn params_to_json_produces_web_search_arguments() {
        let mut params = HashMap::new();
        params.insert("query".to_string(), "{{company}} funding rounds".to_string());
        let mut vars = HashMap::new();
        vars.insert("company".to_string(), "Stripe".to_string());

        let json = params_to_json(&params, &vars);
        assert_eq!(json["query"].as_str(), Some("Stripe funding rounds"));
    }
}
