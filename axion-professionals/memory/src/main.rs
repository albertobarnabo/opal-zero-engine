//! Axion Memory — Wasm Professional
//!
//! Compiled to `wasm32-wasip1`.  The host exposes `/missions` (read-only)
//! via a WASI preopen.  The tool scans that directory for past mission
//! snapshots and returns a human-readable summary.
//!
//! Protocol:
//!   stdin  → JSON: `{"mission_id":"<id>"}` | `{"query":"<text>"}` | `{}`
//!   stdout → plain-text summary on success
//!   stderr → error message + exit(1) on failure

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

// ── Input ─────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct MemoryArgs {
    /// Exact mission ID (e.g. `"mission_1234_plan_trip_rome"`).
    #[serde(default)]
    mission_id: Option<String>,
    /// Free-text query — the tool finds the best-matching past mission.
    #[serde(default)]
    query: Option<String>,
}

// ── Snapshot schema (mirrors axion-core MissionSnapshot) ─────────────────────

#[derive(serde::Deserialize)]
struct Snapshot {
    id: String,
    intent: String,
    status: String,
    task_count: usize,
    #[serde(default)]
    layout_hint: String,
    context: ContextBus,
}

#[derive(serde::Deserialize, Default)]
struct ContextBus {
    #[serde(default)]
    data: HashMap<String, String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const MISSIONS_DIR: &str = "/missions";
/// Maximum characters shown per context value to keep output readable.
const VALUE_PREVIEW_LEN: usize = 300;

fn summarize(s: &Snapshot) -> String {
    let mut out = format!(
        "Mission: {id}\nStatus:  {status}\nIntent:  {intent}\nTasks:   {tasks}\nLayout:  {layout}\n",
        id     = s.id,
        status = s.status,
        intent = s.intent,
        tasks  = s.task_count,
        layout = s.layout_hint,
    );

    if s.context.data.is_empty() {
        out.push_str("Context: (empty)\n");
    } else {
        out.push_str("\nContext keys and values:\n");
        let mut entries: Vec<_> = s.context.data.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str()); // deterministic order
        for (k, v) in &entries {
            let preview = if v.len() > VALUE_PREVIEW_LEN {
                format!("{}…", &v[..VALUE_PREVIEW_LEN])
            } else {
                v.to_string()
            };
            out.push_str(&format!("  {}: {}\n", k, preview));
        }
    }

    out
}

fn list_all(dir: &Path) -> Result<String, String> {
    let entries = json_files(dir)?;
    if entries.is_empty() {
        return Ok("No past missions found.".to_string());
    }
    let mut out = format!("Found {} past mission(s):\n", entries.len());
    for path in &entries {
        if let Ok(s) = load_snapshot(path) {
            out.push_str(&format!("  • {} [{}]: {}\n", s.id, s.status, s.intent));
        }
    }
    Ok(out)
}

/// Return paths of all `*.json` files in `dir`.
fn json_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let read = fs::read_dir(dir)
        .map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
    let mut paths: Vec<_> = read
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .map(|e| e.path())
        .collect();
    paths.sort(); // chronological (filenames start with timestamp)
    Ok(paths)
}

fn load_snapshot(path: &Path) -> Result<Snapshot, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("Read error on {:?}: {}", path, e))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("Parse error in {:?}: {}", path, e))
}

// ── Core logic ────────────────────────────────────────────────────────────────

fn recall(args: &MemoryArgs) -> Result<String, String> {
    let dir = Path::new(MISSIONS_DIR);

    if !dir.exists() {
        return Ok(
            "No missions directory found — no past missions have been saved yet.".to_string(),
        );
    }

    // ── Exact lookup by mission_id ─────────────────────────────────────────
    if let Some(ref id) = args.mission_id {
        let path = dir.join(format!("{}.json", id));
        if !path.exists() {
            // Graceful: also list what IS available
            let listing = list_all(dir).unwrap_or_default();
            return Ok(format!(
                "Mission '{}' not found.\n\n{}",
                id, listing
            ));
        }
        return Ok(summarize(&load_snapshot(&path)?));
    }

    // ── Fuzzy match by query ───────────────────────────────────────────────
    if let Some(ref query) = args.query {
        let q = query.to_lowercase();
        let paths = json_files(dir)?;

        if paths.is_empty() {
            return Ok("No past missions found.".to_string());
        }

        let mut best: Option<(Snapshot, usize)> = None;

        for path in &paths {
            let snapshot = match load_snapshot(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // Score = number of query words that appear in the intent or id.
            let intent_lc = snapshot.intent.to_lowercase();
            let id_lc = snapshot.id.to_lowercase();
            let score = q
                .split_whitespace()
                .filter(|w| intent_lc.contains(w) || id_lc.contains(w))
                .count();

            if score > 0 {
                let is_better = best.as_ref().map_or(true, |(_, s)| score > *s);
                if is_better {
                    best = Some((snapshot, score));
                }
            }
        }

        return match best {
            Some((snapshot, _)) => Ok(summarize(&snapshot)),
            None => {
                // No match — still list what's available so the agent can pivot
                let mut out = format!("No missions matching '{}' found.\n\n", query);
                out.push_str(&list_all(dir).unwrap_or_default());
                Ok(out)
            }
        };
    }

    // ── No arguments — list everything ────────────────────────────────────
    list_all(dir)
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("Failed to read stdin");

    let args: MemoryArgs = match serde_json::from_str(input.trim()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Memory tool: failed to parse arguments: {}", e);
            std::process::exit(1);
        }
    };

    match recall(&args) {
        Ok(result) => print!("{}", result),
        Err(e) => {
            eprintln!("Memory tool error: {}", e);
            std::process::exit(1);
        }
    }
}
