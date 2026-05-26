// Persistent cross-mission memory backed by memory/global.json

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── Limits ────────────────────────────────────────────────────────────────────

const MAX_ENTRIES: usize = 500;
/// Max allowed length (in Unicode scalar values) for a stored value.
const MAX_VALUE_CHARS: usize = 2_000;
/// Suffix appended when a value is silently truncated.
const TRUNCATED_SUFFIX: &str = "[truncated]";
const MAX_KEY_CHARS: usize = 64;

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single persisted fact.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemoryEntry {
    pub value: String,
    /// Identifier of the task/mission that wrote this entry.
    pub written_by: String,
    /// Unix timestamp (seconds) when this entry was last written.
    pub written_at: u64,
}

/// On-disk representation of `memory/global.json`.
#[derive(Serialize, Deserialize, Default)]
struct MemoryFile {
    entries: HashMap<String, MemoryEntry>,
}

/// Lightweight handle to the persistent memory store.
pub struct MemoryStore {
    /// Absolute (or CWD-relative) path to `memory/global.json`.
    path: PathBuf,
}

// ── Implementation ────────────────────────────────────────────────────────────

impl MemoryStore {
    /// Create a handle rooted at `base_dir` (e.g. `Path::new("memory")`).
    ///
    /// Creates the directory if it does not already exist.
    /// Does **not** create the JSON file — that happens lazily on the first write.
    pub fn new(base_dir: &Path) -> Self {
        // Best-effort directory creation; errors are ignored here and will
        // surface as I/O errors on the first write.
        let _ = std::fs::create_dir_all(base_dir);
        MemoryStore {
            path: base_dir.join("global.json"),
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Load the memory file from disk.  Returns an empty `MemoryFile` when the
    /// file is absent or cannot be parsed.
    fn load(&self) -> MemoryFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Upsert `key` → `value` in the store, recording `mission_id` as the author.
    ///
    /// Key rules: alphanumeric + `_` + `-`, max 64 chars.
    /// Value rules: truncated silently to 2000 chars (with `[truncated]` suffix).
    /// Capacity limit: 500 entries maximum (returns `Err` when full and key is new).
    pub fn write(&self, key: &str, value: &str, mission_id: &str) -> Result<(), String> {
        // ── Key validation ────────────────────────────────────────────────────
        if key.is_empty() || key.len() > MAX_KEY_CHARS {
            return Err(format!(
                "Memory key '{}' must be 1–{} characters",
                key, MAX_KEY_CHARS
            ));
        }
        if !key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!(
                "Memory key '{}' may only contain alphanumeric characters, underscores, \
                 and hyphens",
                key
            ));
        }

        // ── Value truncation (silent) ─────────────────────────────────────────
        let value: String = if value.chars().count() > MAX_VALUE_CHARS {
            let keep = MAX_VALUE_CHARS.saturating_sub(TRUNCATED_SUFFIX.len());
            let s: String = value.chars().take(keep).collect();
            format!("{}{}", s, TRUNCATED_SUFFIX)
        } else {
            value.to_string()
        };

        // ── Load, validate capacity, upsert ──────────────────────────────────
        let mut mem = self.load();

        if !mem.entries.contains_key(key) && mem.entries.len() >= MAX_ENTRIES {
            return Err("memory store full".into());
        }

        let written_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        mem.entries.insert(
            key.to_string(),
            MemoryEntry {
                value,
                written_by: mission_id.to_string(),
                written_at,
            },
        );

        // ── Atomic write: .tmp → rename ───────────────────────────────────────
        let serialized =
            serde_json::to_string_pretty(&mem).map_err(|e| e.to_string())?;
        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &serialized).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp_path, &self.path).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Read a single entry by key.  Returns `None` if absent.
    pub fn read(&self, key: &str) -> Option<MemoryEntry> {
        self.load().entries.remove(key)
    }

    /// Return all entries.  Returns an empty map when the file is absent.
    pub fn read_all(&self) -> HashMap<String, MemoryEntry> {
        self.load().entries
    }
}
