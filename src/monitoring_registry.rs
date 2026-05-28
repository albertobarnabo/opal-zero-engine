/// Registry for monitoring manifests.
///
/// A monitoring manifest is a TOML file in `professionals/monitoring/` that
/// defines an autonomous watch strategy: what to research on each run, how
/// often to run it, what structured output the Analyst must produce, and which
/// output fields are most significant for change detection.
///
/// Unlike research manifests (triggered by intent matching), monitoring
/// manifests are addressed by ID and run on a cron schedule.
///
/// # Execution flow
///
/// 1. The server loads all manifests at startup via [`init_default`].
/// 2. A cron job (or a manual API call) looks up a manifest by ID.
/// 3. The server builds a [`Plan`] from [`MonitoringManifest::build_intent`],
///    injects [`MonitoringManifest::output_schema_json`] as `CTX_OUTPUT_SCHEMA`,
///    and calls [`run_monitoring_mission`].
/// 4. The kernel collects data, the Analyst produces structured JSON that matches
///    the manifest's schema, the diff engine compares it to the last run, and a
///    [`MonitoringReport`] is persisted and emitted over SSE.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

// ── Global registry ───────────────────────────────────────────────────────────

static REGISTRY: OnceLock<Vec<MonitoringManifest>> = OnceLock::new();

// ── Structs ───────────────────────────────────────────────────────────────────

/// A fully parsed monitoring manifest.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct MonitoringManifest {
    pub manifest:       MonitoringManifestMeta,
    /// Keys the Analyst must produce each run. Values are type hints
    /// (`"string"`, `"object"`, `"array"`, `"boolean"`, `"number"`).
    #[serde(default)]
    pub output_schema:  HashMap<String, String>,
    /// Fields that are considered significant for change detection.
    /// When non-empty, the frontend/server can highlight changes in these
    /// fields differently from minor noise changes.
    /// When empty, all output fields are treated equally.
    #[serde(default)]
    pub compare_fields: Vec<CompareField>,
}

/// Core metadata for a monitoring manifest.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct MonitoringManifestMeta {
    /// Stable identifier — used as the `monitor_id` in [`MonitoringReport`]
    /// and as the key for the server API (`GET /monitors/:id/reports`).
    pub id:          String,
    /// Human-readable description shown in the dashboard.
    pub description: String,
    /// Cron expression controlling how often this monitor runs.
    /// Standard 5-field format: `"0 8 * * *"` = daily at 8 am UTC.
    pub schedule:    String,
    /// Full intent sent to the Planner on each run.
    /// Should be specific enough that the Analyst produces the keys in
    /// `output_schema` without extra guidance.
    pub intent:      String,
}

/// A single field in the output schema that is watched for changes.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct CompareField {
    /// The top-level key in `output_schema` to watch.
    pub key:         String,
    /// Human-readable description of what this field represents.
    #[serde(default)]
    pub description: String,
    /// Signal strength: `"high"` | `"medium"` | `"low"`.
    /// The frontend uses this to visually emphasise important changes.
    #[serde(default = "default_importance")]
    pub importance:  String,
}

fn default_importance() -> String { "medium".to_string() }

// ── MonitoringManifest helpers ────────────────────────────────────────────────

impl MonitoringManifest {
    /// Returns the manifest's intent — the string passed to the Planner.
    pub fn intent(&self) -> &str {
        &self.manifest.intent
    }

    /// Serialise `output_schema` to JSON for injection as `CTX_OUTPUT_SCHEMA`.
    ///
    /// Returns `None` when the manifest has no schema (the Analyst will use its
    /// default output format).
    pub fn output_schema_json(&self) -> Option<String> {
        if self.output_schema.is_empty() {
            return None;
        }
        serde_json::to_string(&self.output_schema).ok()
    }

    /// Returns the set of field keys marked as high-importance for diffing.
    /// An empty slice means all fields are treated equally.
    pub fn high_importance_fields(&self) -> Vec<&str> {
        self.compare_fields.iter()
            .filter(|f| f.importance == "high")
            .map(|f| f.key.as_str())
            .collect()
    }
}

// ── Registry initialisation ───────────────────────────────────────────────────

/// Load monitoring manifests from `dir`. Safe to call multiple times — only
/// the first call has any effect (subsequent calls are no-ops).
pub fn init(dir: &Path) {
    if REGISTRY.get().is_some() {
        return;
    }
    let manifests = load_from_dir(dir);
    tracing::info!(count = manifests.len(), "MonitoringRegistry: manifests loaded");
    let _ = REGISTRY.set(manifests);
}

/// Try several path strategies to locate `professionals/monitoring/` and
/// initialise the registry. Called once at server startup.
pub fn init_default() {
    let compile_time = concat!(env!("CARGO_MANIFEST_DIR"), "/professionals/monitoring");
    if Path::new(compile_time).is_dir() {
        return init(Path::new(compile_time));
    }

    // Workspace-relative path (when called from opalzero-server)
    let ws_path = std::path::PathBuf::from(compile_time)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("opalzero-core/professionals/monitoring"));
    if let Some(path) = ws_path {
        if path.is_dir() {
            return init(&path);
        }
    }

    // CWD fallback
    init(Path::new("professionals/monitoring"));
}

// ── Public interface ──────────────────────────────────────────────────────────

/// Return all loaded monitoring manifests.
/// Returns an empty slice when the registry has not been initialised.
pub fn get_all() -> &'static [MonitoringManifest] {
    REGISTRY.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Look up a monitoring manifest by its `id` field.
pub fn get_by_id(id: &str) -> Option<&'static MonitoringManifest> {
    get_all().iter().find(|m| m.manifest.id == id)
}

// ── File loading ──────────────────────────────────────────────────────────────

fn load_from_dir(dir: &Path) -> Vec<MonitoringManifest> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        tracing::warn!(dir = ?dir, "MonitoringRegistry: directory not found — no manifests loaded");
        return vec![];
    };

    let mut manifests = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<MonitoringManifest>(&content) {
                Ok(m) => {
                    tracing::debug!(id = %m.manifest.id, schedule = %m.manifest.schedule, "MonitoringRegistry: loaded");
                    manifests.push(m);
                }
                Err(e) => {
                    tracing::warn!(path = ?path, error = %e, "MonitoringRegistry: parse error — skipping");
                }
            },
            Err(e) => {
                tracing::warn!(path = ?path, error = %e, "MonitoringRegistry: read error — skipping");
            }
        }
    }
    manifests
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_toml(s: &str) -> MonitoringManifest {
        toml::from_str(s).expect("TOML should parse cleanly")
    }

    #[test]
    fn minimal_manifest_parses() {
        let m = parse_toml(r#"
            [manifest]
            id          = "test_monitor"
            description = "A test monitor"
            schedule    = "0 8 * * *"
            intent      = "Research something"
        "#);
        assert_eq!(m.manifest.id, "test_monitor");
        assert_eq!(m.manifest.schedule, "0 8 * * *");
        assert!(m.output_schema.is_empty());
        assert!(m.compare_fields.is_empty());
    }

    #[test]
    fn full_manifest_parses() {
        let m = parse_toml(r#"
            [manifest]
            id          = "ai_pricing"
            description = "Track AI pricing"
            schedule    = "0 8 * * *"
            intent      = "Research AI API pricing"

            [output_schema]
            providers = "object"
            free_tiers = "object"
            recent_changes = "string"

            [[compare_fields]]
            key         = "providers"
            description = "Provider pricing"
            importance  = "high"

            [[compare_fields]]
            key         = "free_tiers"
            description = "Free tier availability"
            importance  = "medium"
        "#);
        assert_eq!(m.manifest.id, "ai_pricing");
        assert_eq!(m.output_schema.len(), 3);
        assert_eq!(m.compare_fields.len(), 2);
        assert_eq!(m.compare_fields[0].importance, "high");
    }

    #[test]
    fn output_schema_json_returns_none_when_empty() {
        let m = parse_toml(r#"
            [manifest]
            id          = "x"
            description = "x"
            schedule    = "* * * * *"
            intent      = "x"
        "#);
        assert!(m.output_schema_json().is_none());
    }

    #[test]
    fn output_schema_json_is_valid_json() {
        let m = parse_toml(r#"
            [manifest]
            id          = "x"
            description = "x"
            schedule    = "* * * * *"
            intent      = "x"

            [output_schema]
            pricing = "object"
            summary = "string"
        "#);
        let json_str = m.output_schema_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["pricing"], "object");
        assert_eq!(parsed["summary"], "string");
    }

    #[test]
    fn high_importance_fields_filtered_correctly() {
        let m = parse_toml(r#"
            [manifest]
            id          = "x"
            description = "x"
            schedule    = "* * * * *"
            intent      = "x"

            [[compare_fields]]
            key        = "a"
            importance = "high"

            [[compare_fields]]
            key        = "b"
            importance = "medium"

            [[compare_fields]]
            key        = "c"
            importance = "high"
        "#);
        let hi = m.high_importance_fields();
        assert_eq!(hi.len(), 2);
        assert!(hi.contains(&"a"));
        assert!(hi.contains(&"c"));
    }

    // File-based parse tests (run from workspace root where files exist)

    #[test]
    fn ai_pricing_toml_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/professionals/monitoring/ai_pricing.toml");
        if !std::path::Path::new(path).exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        let m: MonitoringManifest = toml::from_str(&content).expect("ai_pricing.toml should parse");
        assert_eq!(m.manifest.id, "ai_pricing");
        assert!(!m.manifest.intent.is_empty());
        assert!(!m.output_schema.is_empty());
    }

    #[test]
    fn saas_competitors_toml_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/professionals/monitoring/saas_competitors.toml");
        if !std::path::Path::new(path).exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        let m: MonitoringManifest = toml::from_str(&content).expect("saas_competitors.toml should parse");
        assert_eq!(m.manifest.id, "saas_competitors");
    }

    #[test]
    fn startup_hiring_toml_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/professionals/monitoring/startup_hiring.toml");
        if !std::path::Path::new(path).exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        let m: MonitoringManifest = toml::from_str(&content).expect("startup_hiring.toml should parse");
        assert_eq!(m.manifest.id, "startup_hiring");
    }
}
