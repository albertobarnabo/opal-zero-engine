use crate::planner::Plan;
use crate::protocol::MissionState;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MissionSnapshot {
    pub id: String,
    pub timestamp: u64,
    pub intent: String,
    pub task_count: usize,
    pub expanded_task_count: usize,
    pub status: String,
    /// `"Synthesized"` | `"Analytical"` | `"Itinerary"`.
    #[serde(default)]
    pub layout_hint: String,
    pub context: crate::protocol::ContextBus,
    /// Present when the mission produced a finalized `MissionState` payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_state: Option<MissionState>,
}

/// Load an existing mission snapshot from `missions/<id>.json`.
pub fn load_snapshot(id: &str) -> Result<MissionSnapshot, String> {
    // Basic path-traversal guard: only alphanumerics + underscores.
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return Err(format!("Invalid mission ID: '{}'", id));
    }
    let path = std::path::Path::new("missions").join(format!("{}.json", id));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Mission '{}' not found: {}", id, e))?;
    serde_json::from_str::<MissionSnapshot>(&content)
        .map_err(|e| format!("Failed to deserialise snapshot '{}': {}", id, e))
}

/// Overwrite `missions/<id>.json` with the current plan state.
///
/// Used by the refinement path to update an existing snapshot in-place rather
/// than creating a new file with a generated ID.
pub fn save_snapshot_with_id(
    plan: &Plan,
    original_task_count: usize,
    status: &str,
    id: &str,
) -> Result<(), String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mission_state: Option<MissionState> = plan
        .tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .filter_map(|r| serde_json::from_str::<MissionState>(r).ok())
        .filter(|s| !s.data_payload.is_null())
        .last();

    let layout_hint = if mission_state.is_some() {
        "Synthesized"
    } else if plan.tasks.iter().any(|t| matches!(t.role, crate::protocol::AgentRole::Coder)) {
        "Analytical"
    } else {
        "Itinerary"
    }
    .to_string();

    let snapshot = MissionSnapshot {
        id: id.to_string(),
        timestamp,
        intent: plan.original_intent.clone(),
        task_count: plan.tasks.len(),
        expanded_task_count: plan.tasks.len().saturating_sub(original_task_count),
        status: status.to_string(),
        layout_hint,
        context: plan.context.clone(),
        mission_state,
    };

    let dir = std::path::Path::new("missions");
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create missions/: {}", e))?;

    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;

    // ── Atomic write: temp → fsync → rename ──────────────────────────────────
    // Writing directly to the final path risks a partial/corrupt file if the
    // process is interrupted mid-write.  Write to a sibling `.tmp` file first,
    // fsync it to durable storage, then atomically rename it into place.
    // On POSIX (Linux/macOS) `rename(2)` over an existing file is atomic on
    // the same filesystem, so readers always see either the old or new content.
    let tmp_path   = dir.join(format!("{}.json.tmp", id));
    let final_path = dir.join(format!("{}.json", id));

    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("Failed to create temp file for '{}': {}", id, e))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write temp file for '{}': {}", id, e))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync temp file for '{}': {}", id, e))?;
    }

    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Failed to rename snapshot into place for '{}': {}", id, e))?;

    println!("📁 Snapshot updated: missions/{}.json", id);
    Ok(())
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

    let id = format!("mission_{}_{}", timestamp, crate::util::slugify(&plan.original_intent, 5));

    let mission_state: Option<MissionState> = plan
        .tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .filter_map(|r| serde_json::from_str::<MissionState>(r).ok())
        .filter(|s| !s.data_payload.is_null())
        .last();

    let layout_hint = if mission_state.is_some() {
        "Synthesized"
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
        mission_state,
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

#[cfg(test)]
mod tests {
    use super::*;

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
