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

/// Find and load the most recent **completed** snapshot whose `intent` matches
/// `intent` (case-insensitive, trimmed).
///
/// Scans every `missions/*.json` file (skips `.tmp` files), filters to
/// `status == "completed"` and a normalised intent match, and returns the one
/// with the highest `timestamp`.
///
/// Returns `None` if the `missions/` directory doesn't exist, is empty, or
/// contains no matching completed snapshot.
pub fn load_latest_snapshot_for_intent(intent: &str) -> Option<MissionSnapshot> {
    let dir = std::path::Path::new("missions");
    if !dir.exists() {
        return None;
    }

    let needle = intent.trim().to_lowercase();

    let entries = std::fs::read_dir(dir).ok()?;

    let mut best: Option<MissionSnapshot> = None;

    for entry in entries.flatten() {
        let path = entry.path();

        // Skip non-.json files and .tmp temps.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "json" {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let snapshot: MissionSnapshot = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if snapshot.status != "completed" {
            continue;
        }

        // Normalise intent: strip REFINE[] wrapper if present, then compare.
        let haystack = snapshot
            .intent
            .trim_start_matches("REFINE[")
            .split("]: ")
            .last()
            .unwrap_or(&snapshot.intent)
            .trim()
            .to_lowercase();

        if haystack != needle {
            continue;
        }

        let is_newer = best
            .as_ref()
            .map(|b| snapshot.timestamp > b.timestamp)
            .unwrap_or(true);

        if is_newer {
            best = Some(snapshot);
        }
    }

    best
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

    tracing::info!(mission_id = id, "snapshot updated");
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

    tracing::info!(mission_id = %id, "snapshot saved");
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

    // ── load_latest_snapshot_for_intent ─────────────────────────────────────

    fn write_snapshot_file(snapshot: &MissionSnapshot) -> std::path::PathBuf {
        let dir = std::path::Path::new("missions");
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(format!("{}.json", snapshot.id));
        let json = serde_json::to_string_pretty(snapshot).unwrap();
        std::fs::write(&path, &json).unwrap();
        path
    }

    fn make_snapshot(id: &str, intent: &str, timestamp: u64, status: &str) -> MissionSnapshot {
        MissionSnapshot {
            id: id.to_string(),
            timestamp,
            intent: intent.to_string(),
            task_count: 1,
            expanded_task_count: 0,
            status: status.to_string(),
            layout_hint: "Itinerary".to_string(),
            context: crate::protocol::ContextBus::default(),
            mission_state: None,
        }
    }

    #[test]
    fn load_latest_returns_most_recent_completed_match() {
        let older = make_snapshot("test_older_abc", "track bitcoin price", 1000, "completed");
        let newer = make_snapshot("test_newer_abc", "track bitcoin price", 2000, "completed");
        let p1 = write_snapshot_file(&older);
        let p2 = write_snapshot_file(&newer);

        let result = load_latest_snapshot_for_intent("track bitcoin price");
        let _ = std::fs::remove_file(p1);
        let _ = std::fs::remove_file(p2);

        assert!(result.is_some(), "should find a match");
        assert_eq!(result.unwrap().id, "test_newer_abc");
    }

    #[test]
    fn load_latest_ignores_failed_snapshots() {
        let failed = make_snapshot("test_failed_xyz", "some special intent xyz", 9999, "failed");
        let p = write_snapshot_file(&failed);

        let result = load_latest_snapshot_for_intent("some special intent xyz");
        let _ = std::fs::remove_file(p);

        assert!(result.is_none(), "failed snapshots must be ignored");
    }

    #[test]
    fn load_latest_is_case_insensitive() {
        let snap = make_snapshot("test_case_ci", "Track Bitcoin Price", 1000, "completed");
        let p = write_snapshot_file(&snap);

        let result = load_latest_snapshot_for_intent("track bitcoin price");
        let _ = std::fs::remove_file(p);

        assert!(result.is_some(), "intent matching must be case-insensitive");
    }

    #[test]
    fn load_latest_returns_none_when_no_match() {
        let result = load_latest_snapshot_for_intent("intent that definitely does not exist xyz123");
        assert!(result.is_none());
    }

    #[test]
    fn load_latest_strips_refine_wrapper_for_matching() {
        let refined = make_snapshot(
            "test_refined_wrap",
            "REFINE[track bitcoin price]: add hourly data",
            5000,
            "completed",
        );
        let p = write_snapshot_file(&refined);

        let result = load_latest_snapshot_for_intent("add hourly data");
        let _ = std::fs::remove_file(p);

        assert!(result.is_some(), "REFINE[] wrapper should be stripped before matching");
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
