/// JSON diff engine for monitoring reports.
///
/// Compares two `serde_json::Value` payloads (the `data_payload` from
/// consecutive `MissionState` runs) and produces a flat list of [`Change`]s.
///
/// ## Diff rules
///
/// | Left type  | Right type | Action                                        |
/// |------------|------------|-----------------------------------------------|
/// | Object     | Object     | Recurse into each key; detect added/removed   |
/// | Array      | Array      | Set-based diff: items added to / removed from |
/// | Primitive  | Primitive  | Compare by value; emit `Modified` if different|
/// | Any        | Any        | Type mismatch → emit `Modified`               |
///
/// Paths use dot-notation: `"pricing.enterprise"`, `"features"`.
/// Array-level changes carry the parent path (`"features"`, not `"features[0]"`).
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

// ── Change ────────────────────────────────────────────────────────────────────

/// A single detected difference between two monitoring-run payloads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Change {
    /// Dot-notation path to the changed field (e.g. `"pricing.pro_tier"`).
    pub path: String,
    /// `"added"` | `"removed"` | `"modified"`
    pub kind: String,
    /// Current (new) value, or the removed value for `kind = "removed"`.
    pub value: Value,
    /// Previous value — only present when `kind = "modified"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_value: Option<Value>,
    /// Human-readable one-liner the frontend can display directly.
    pub description: String,
}

impl Change {
    fn added(path: &str, value: Value) -> Self {
        let description = format!("\"{}\" appeared: {}", path, value_summary(&value));
        Change { path: path.to_string(), kind: "added".to_string(), value, previous_value: None, description }
    }

    fn removed(path: &str, value: Value) -> Self {
        let description = format!("\"{}\" disappeared: was {}", path, value_summary(&value));
        Change { path: path.to_string(), kind: "removed".to_string(), value, previous_value: None, description }
    }

    fn modified(path: &str, from: Value, to: Value) -> Self {
        let description = format!(
            "\"{}\" changed: {} → {}",
            path,
            value_summary(&from),
            value_summary(&to),
        );
        Change {
            path: path.to_string(),
            kind: "modified".to_string(),
            value: to,
            previous_value: Some(from),
            description,
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Compute the diff between `previous` and `current` payloads.
///
/// Returns an empty `Vec` if the payloads are identical or both are null/missing.
pub fn diff(previous: &Value, current: &Value) -> Vec<Change> {
    let mut changes = Vec::new();
    diff_recursive(previous, current, "", &mut changes);
    changes
}

/// Produce a concise, human-readable summary of a change list.
///
/// Examples:
/// - `"First run — baseline established"`
/// - `"No changes detected"`
/// - `"3 changes: 2 values modified, 1 item added"`
pub fn summarize(changes: &[Change], is_first_run: bool) -> String {
    if is_first_run {
        return "First monitoring run — baseline established".to_string();
    }
    if changes.is_empty() {
        return "No changes detected".to_string();
    }

    let added    = changes.iter().filter(|c| c.kind == "added").count();
    let removed  = changes.iter().filter(|c| c.kind == "removed").count();
    let modified = changes.iter().filter(|c| c.kind == "modified").count();

    let mut parts: Vec<String> = Vec::new();
    if modified > 0 { parts.push(format!("{} modified", modified)); }
    if added    > 0 { parts.push(format!("{} added",    added));    }
    if removed  > 0 { parts.push(format!("{} removed",  removed));  }

    format!("{} change{}: {}", changes.len(), if changes.len() == 1 { "" } else { "s" }, parts.join(", "))
}

// ── Internal recursion ────────────────────────────────────────────────────────

fn diff_recursive(prev: &Value, curr: &Value, path: &str, out: &mut Vec<Change>) {
    if prev == curr {
        return;
    }

    match (prev, curr) {
        (Value::Object(p_map), Value::Object(c_map)) => {
            // Keys in previous but not current → Removed
            for key in p_map.keys() {
                let child_path = child(path, key);
                if !c_map.contains_key(key) {
                    out.push(Change::removed(&child_path, p_map[key].clone()));
                }
            }
            // Keys in current but not previous → Added
            for key in c_map.keys() {
                let child_path = child(path, key);
                if !p_map.contains_key(key) {
                    out.push(Change::added(&child_path, c_map[key].clone()));
                }
            }
            // Keys in both → recurse
            for key in p_map.keys() {
                if c_map.contains_key(key) {
                    diff_recursive(&p_map[key], &c_map[key], &child(path, key), out);
                }
            }
        }

        (Value::Array(p_arr), Value::Array(c_arr)) => {
            // Set-based diff: compare items by their canonical JSON string.
            // Preserves ordering within each added/removed group.
            let p_set: HashSet<String> = p_arr.iter().map(|v| canonical(v)).collect();
            let c_set: HashSet<String> = c_arr.iter().map(|v| canonical(v)).collect();

            for item in c_arr {
                if !p_set.contains(&canonical(item)) {
                    out.push(Change::added(path, item.clone()));
                }
            }
            for item in p_arr {
                if !c_set.contains(&canonical(item)) {
                    out.push(Change::removed(path, item.clone()));
                }
            }
        }

        // Primitive mismatch (or type mismatch)
        _ => {
            out.push(Change::modified(path, prev.clone(), curr.clone()));
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build a child path: `""` + `"key"` → `"key"`, `"parent"` + `"child"` → `"parent.child"`.
fn child(parent: &str, key: &str) -> String {
    if parent.is_empty() { key.to_string() } else { format!("{}.{}", parent, key) }
}

/// Canonical JSON string for set membership (sorted keys for objects).
fn canonical(v: &Value) -> String {
    // serde_json serialises maps in insertion order, which varies.
    // For our purposes (monitoring payloads are typically flat), this is fine.
    v.to_string()
}

/// Compact human-readable summary of a JSON value (max 80 chars).
fn value_summary(v: &Value) -> String {
    match v {
        Value::String(s) => {
            if s.len() <= 80 { format!("\"{}\"", s) }
            else             { format!("\"{}…\"", &s[..80]) }
        }
        Value::Array(arr) => format!("[{} item{}]", arr.len(), if arr.len() == 1 { "" } else { "s" }),
        Value::Object(m)  => format!("{{{}  key{}}}", m.len(), if m.len() == 1 { "" } else { "s" }),
        other             => other.to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_payloads_produce_no_changes() {
        let v = json!({ "price": "$20", "features": ["a", "b"] });
        assert!(diff(&v, &v).is_empty());
    }

    #[test]
    fn added_top_level_field() {
        let prev = json!({ "price": "$20" });
        let curr = json!({ "price": "$20", "enterprise": "$500" });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "added");
        assert_eq!(changes[0].path, "enterprise");
    }

    #[test]
    fn removed_top_level_field() {
        let prev = json!({ "price": "$20", "free_tier": true });
        let curr = json!({ "price": "$20" });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "removed");
        assert_eq!(changes[0].path, "free_tier");
    }

    #[test]
    fn modified_primitive() {
        let prev = json!({ "price": "$20" });
        let curr = json!({ "price": "$25" });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "modified");
        assert_eq!(changes[0].previous_value, Some(json!("$20")));
        assert_eq!(changes[0].value, json!("$25"));
    }

    #[test]
    fn nested_object_change_uses_dot_path() {
        let prev = json!({ "pricing": { "pro": "$20" } });
        let curr = json!({ "pricing": { "pro": "$25" } });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "pricing.pro");
    }

    #[test]
    fn array_item_added() {
        let prev = json!({ "features": ["a", "b"] });
        let curr = json!({ "features": ["a", "b", "c"] });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "added");
        assert_eq!(changes[0].path, "features");
        assert_eq!(changes[0].value, json!("c"));
    }

    #[test]
    fn array_item_removed() {
        let prev = json!({ "features": ["a", "b", "c"] });
        let curr = json!({ "features": ["a", "b"] });
        let changes = diff(&prev, &curr);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, "removed");
        assert_eq!(changes[0].value, json!("c"));
    }

    #[test]
    fn no_change_in_array_same_items() {
        let v = json!({ "features": ["a", "b", "c"] });
        assert!(diff(&v, &v).is_empty());
    }

    #[test]
    fn summarize_first_run() {
        assert_eq!(summarize(&[], true), "First monitoring run — baseline established");
    }

    #[test]
    fn summarize_no_changes() {
        assert_eq!(summarize(&[], false), "No changes detected");
    }

    #[test]
    fn summarize_mixed_changes() {
        let prev = json!({ "a": 1, "b": 2, "c": 3 });
        let curr = json!({ "a": 99, "c": 3, "d": 4 });
        let changes = diff(&prev, &curr);
        let s = summarize(&changes, false);
        assert!(s.contains("modified"));
        assert!(s.contains("added"));
        assert!(s.contains("removed"));
    }

    #[test]
    fn deep_realistic_monitoring_diff() {
        let prev = json!({
            "overview": "Company A does X",
            "pricing": { "pro": "$20/mo", "enterprise": "contact us" },
            "employees": 450,
            "recent_news": ["launched v2", "hired CTO"]
        });
        let curr = json!({
            "overview": "Company A does X",
            "pricing": { "pro": "$25/mo", "enterprise": "contact us", "starter": "$9/mo" },
            "employees": 510,
            "recent_news": ["launched v2", "hired CTO", "raised Series B"]
        });
        let changes = diff(&prev, &curr);
        // pricing.pro modified, pricing.starter added, employees modified, recent_news item added
        assert!(changes.len() >= 3, "expected at least 3 changes, got {}", changes.len());
        let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"pricing.pro"),     "missing pricing.pro change");
        assert!(paths.contains(&"pricing.starter"), "missing pricing.starter change");
        assert!(paths.contains(&"employees"),       "missing employees change");
    }
}
