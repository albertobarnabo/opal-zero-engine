use crate::planner::Plan;
use crate::protocol::UIBlueprint;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MissionSnapshot {
    pub id: String,
    pub timestamp: u64,
    pub intent: String,
    pub task_count: usize,
    pub expanded_task_count: usize,
    pub status: String,
    /// `"Designed"` | `"Analytical"` | `"Itinerary"`.
    #[serde(default)]
    pub layout_hint: String,
    pub context: crate::protocol::ContextBus,
    /// Present when the mission produced a `build_dynamic_ui` blueprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_blueprint: Option<UIBlueprint>,
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

    let ui_blueprint: Option<UIBlueprint> = plan
        .tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .filter_map(|r| serde_json::from_str::<UIBlueprint>(r).ok())
        .filter(|bp| !bp.components.is_empty())
        .last();

    let layout_hint = if ui_blueprint.is_some() {
        "Designed"
    } else if plan.tasks.iter().any(|t| matches!(t.role, crate::protocol::AgentRole::Coder)) {
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
        ui_blueprint,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── slugify ──────────────────────────────────────────────────────────────

    #[test]
    fn slugify_empty_string_returns_mission() {
        assert_eq!(slugify("", 5), "mission");
    }

    #[test]
    fn slugify_all_stop_words_returns_mission() {
        assert_eq!(slugify("a the is an in on at to for of", 5), "mission");
    }

    #[test]
    fn slugify_emoji_only_returns_mission() {
        assert_eq!(slugify("🚀🌍✨", 5), "mission");
    }

    #[test]
    fn slugify_max_words_respected() {
        let result = slugify("find cheap flights from london to rome paris berlin", 3);
        let parts: Vec<&str> = result.split('_').collect();
        assert!(parts.len() <= 3, "Got {} words: {:?}", parts.len(), parts);
    }

    #[test]
    fn slugify_very_long_input_capped() {
        let long = "word1 word2 word3 word4 word5 word6 word7 word8 word9 word10";
        let result = slugify(long, 5);
        let parts: Vec<&str> = result.split('_').collect();
        assert_eq!(parts.len(), 5);
    }

    #[test]
    fn slugify_lowercases_all_words() {
        let result = slugify("FIND FLIGHTS ROME", 5);
        assert_eq!(result, result.to_lowercase());
    }

    #[test]
    fn slugify_filters_stop_words_from_output() {
        let result = slugify("find the best hotels in rome", 5);
        let parts: Vec<&str> = result.split('_').collect();
        assert!(!parts.contains(&"the"), "Stop word 'the' should be removed");
        assert!(!parts.contains(&"in"), "Stop word 'in' should be removed");
    }

    #[test]
    fn slugify_single_significant_word() {
        let result = slugify("rome", 5);
        assert_eq!(result, "rome");
    }

    // ── save_snapshot ────────────────────────────────────────────────────────

    #[test]
    fn save_snapshot_creates_missions_dir_and_returns_id() {
        use crate::planner::Plan;
        use crate::protocol::AgentRole;

        let mut plan = Plan::new("Test snapshot mission");
        plan.add_task("Do something for the test", vec![], AgentRole::Analyst);

        let result = save_snapshot(&plan, 1, "completed");
        assert!(result.is_ok(), "save_snapshot returned Err: {:?}", result);

        let id = result.unwrap();
        assert!(id.starts_with("mission_"), "ID should start with 'mission_', got: {}", id);

        let path = std::path::Path::new("missions").join(format!("{}.json", id));
        assert!(path.exists(), "Snapshot file missing at {:?}", path);

        // Cleanup so successive test runs stay clean.
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_snapshot_sets_analytical_hint_for_coder_tasks() {
        use crate::planner::Plan;
        use crate::protocol::AgentRole;

        let mut plan = Plan::new("Coder mission");
        plan.add_task("Write a Python script", vec![], AgentRole::Coder);

        let id = save_snapshot(&plan, 1, "completed").unwrap();
        let path = std::path::Path::new("missions").join(format!("{}.json", id));
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert!(content.contains("\"Analytical\""), "layout_hint should be 'Analytical' when Coder is present");
    }

    #[test]
    fn save_snapshot_sets_itinerary_hint_for_non_coder_tasks() {
        use crate::planner::Plan;
        use crate::protocol::AgentRole;

        let mut plan = Plan::new("Itinerary mission");
        plan.add_task("Search for flights", vec![], AgentRole::WebSearcher);

        let id = save_snapshot(&plan, 1, "completed").unwrap();
        let path = std::path::Path::new("missions").join(format!("{}.json", id));
        let content = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert!(content.contains("\"Itinerary\""), "layout_hint should be 'Itinerary' without Coder tasks");
    }
}
