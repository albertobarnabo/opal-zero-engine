use crate::planner::Plan;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MissionSnapshot {
    pub id: String,
    pub timestamp: u64,
    pub intent: String,
    pub task_count: usize,
    pub expanded_task_count: usize,
    pub status: String,
    /// `"Analytical"` if a Coder agent ran Python; `"Itinerary"` otherwise.
    #[serde(default)]
    pub layout_hint: String,
    pub context: crate::protocol::ContextBus,
}

/// Write the completed plan to `missions/<id>.json`. Returns the mission ID.
/// Errors are non-fatal — caller should log and continue.
pub fn save_snapshot(
    plan: &Plan,
    original_task_count: usize,
    status: &str,
) -> Result<String, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let id = format!("mission_{}_{}", timestamp, slugify(&plan.original_intent, 5));

    let layout_hint = if plan.tasks.iter().any(|t| {
        matches!(t.role, crate::protocol::AgentRole::Coder)
    }) {
        "Analytical"
    } else {
        "Itinerary"
    }
    .to_string();

    let snapshot = MissionSnapshot {
        id: id.clone(),
        timestamp,
        intent: plan.original_intent.clone(),
        task_count: plan.tasks.len(),
        expanded_task_count: plan.tasks.len().saturating_sub(original_task_count),
        status: status.to_string(),
        layout_hint,
        context: plan.context.clone(),
    };

    let dir = std::path::Path::new("missions");
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create missions/: {}", e))?;

    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;
    std::fs::write(dir.join(format!("{}.json", id)), &json)
        .map_err(|e| format!("Failed to write snapshot: {}", e))?;

    println!("📁 Snapshot saved: missions/{}.json", id);
    Ok(id)
}

/// Derive a compact, filename-safe slug from arbitrary text.
/// Lowercases, strips stop-words, takes up to `max_words` tokens, joins with `_`.
pub fn slugify(text: &str, max_words: usize) -> String {
    const STOP: &[&str] = &[
        "a", "an", "the", "in", "on", "at", "to", "for", "of", "and", "or",
        "is", "are", "be", "this", "that", "it", "with", "from", "by",
    ];

    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()) && w.len() > 1)
        .take(max_words)
        .collect();

    if words.is_empty() {
        return "mission".to_string();
    }
    words.join("_")
}
