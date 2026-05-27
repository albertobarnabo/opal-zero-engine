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
//! If no manifest matches the intent, or if the manifest path errors, the
//! Dispatcher falls through to the normal Exa multi-turn search.

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

/// A single data source within a research manifest.
#[derive(Debug, Deserialize)]
pub struct SourceSpec {
    pub id:   String,
    /// Native tool name — must match an arm in `execute_source`.
    pub tool: String,
    #[serde(default)]
    pub description: String,
    /// Key/value params forwarded to the tool (e.g. `period = "compact"`).
    #[serde(default)]
    pub params: HashMap<String, String>,
    /// Abort the manifest path when `true` and this source errors.
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

// ── Source execution ──────────────────────────────────────────────────────────
//
// Routes each source spec to the corresponding native Rust tool via the
// standard `execute_tool` interface.  Using `execute_tool` keeps key
// resolution, registry validation, and error formatting consistent with the
// normal agent path.

async fn execute_source(
    source: &SourceSpec,
    vars:   &HashMap<String, String>,
    keys:   &RequestKeys,
) -> Result<String, String> {
    let ticker = vars.get("ticker").map(|s| s.as_str()).unwrap_or("");

    let arguments = match source.tool.as_str() {
        "get_company_overview" => {
            serde_json::json!({ "symbol": ticker }).to_string()
        }
        "get_price_history" => {
            let period = source.params.get("period").map(|s| s.as_str()).unwrap_or("compact");
            serde_json::json!({ "symbol": ticker, "period": period }).to_string()
        }
        "get_income_statement" => {
            serde_json::json!({ "symbol": ticker }).to_string()
        }
        "get_news_sentiment" => {
            serde_json::json!({ "symbol": ticker }).to_string()
        }
        unknown => return Err(format!("ResearchManifest: unknown source tool '{unknown}'")),
    };

    crate::tools::execute_tool(&source.tool, &arguments, "research-manifest", keys).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: build a minimal ResearchManifest in memory (no disk I/O).
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

    #[test]
    fn manifest_deserializes() {
        let m = equity_manifest();
        assert_eq!(m.manifest.id, "equity_research");
        assert_eq!(m.sources.len(), 2);
        assert!(m.variables.contains_key("ticker"));
    }

    #[test]
    fn regex_extracts_dollar_prefixed_ticker() {
        let spec = VarSpec {
            regex:    Some(r"\$([A-Z]{1,5})|\b([A-Z]{2,5})\b".to_string()),
            hint:     "ticker".to_string(),
            required: true,
        };
        let re = regex::Regex::new(spec.regex.as_ref().unwrap()).unwrap();
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
        let matched = m.manifest.match_patterns.iter().any(|p| lower.contains(&p.to_lowercase()));
        assert!(matched, "should match 'stock'");

        let no_match = "write a poem about the moon".to_lowercase();
        let not_matched = m.manifest.match_patterns.iter().any(|p| no_match.contains(&p.to_lowercase()));
        assert!(!not_matched, "should not match");
    }
}
