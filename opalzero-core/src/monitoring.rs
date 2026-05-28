/// Monitoring reports: persistence and loading.
///
/// A [`MonitoringReport`] is produced once per `run_monitoring_mission` call.
/// It records what the current run found, what the previous run found, and the
/// structured diff between them.
///
/// Reports are stored in `monitoring_reports/<monitor_id>_<timestamp>.json`.
/// The directory is created on first write.  Loading returns reports in
/// descending-timestamp order (most recent first).
use crate::diff_engine::Change;
use serde::{Deserialize, Serialize};

// ── MonitoringReport ──────────────────────────────────────────────────────────

/// The structured output of a single monitoring run.
///
/// Emitted via `MissionUpdate::MonitoringReport` over the SSE channel AND
/// persisted to `monitoring_reports/` for the server API to serve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringReport {
    /// Stable identifier for this monitor (slug derived from the intent).
    pub monitor_id: String,
    /// The original intent string that drives the monitoring mission.
    pub intent: String,
    /// Unix timestamp (seconds) for this run.
    pub timestamp: u64,
    /// Unix timestamp of the immediately preceding completed run, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_run_timestamp: Option<u64>,
    /// Whether any differences were detected since the last run.
    pub has_changes: bool,
    /// One-liner the frontend/notification system can display directly.
    /// E.g. `"3 changes: 2 values modified, 1 item added"`.
    pub change_summary: String,
    /// Structured list of individual changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<Change>,
    /// The full structured payload produced by this run.
    pub current_payload: serde_json::Value,
    /// The full structured payload from the previous run (omitted on first run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_payload: Option<serde_json::Value>,
}

// ── Persistence ───────────────────────────────────────────────────────────────

const REPORTS_DIR: &str = "monitoring_reports";

/// Persist a monitoring report to `monitoring_reports/<id>_<ts>.json`.
///
/// Uses the same atomic tmp→rename pattern as mission snapshots so the file
/// is never partially written.
pub fn save_report(report: &MonitoringReport) -> Result<(), String> {
    let dir = std::path::Path::new(REPORTS_DIR);
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("Failed to create {REPORTS_DIR}/: {e}"))?;

    let filename = format!("{}_{}.json", report.monitor_id, report.timestamp);
    let tmp_path   = dir.join(format!("{filename}.tmp"));
    let final_path = dir.join(&filename);

    let json = serde_json::to_string_pretty(report)
        .map_err(|e| format!("JSON serialisation failed: {e}"))?;

    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("Failed to create temp file: {e}"))?;
        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write temp file: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("Failed to sync temp file: {e}"))?;
    }

    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("Failed to rename report into place: {e}"))?;

    tracing::info!(monitor_id = %report.monitor_id, has_changes = report.has_changes, changes = report.changes.len(), "monitoring report saved");
    Ok(())
}

/// Load all reports for a given monitor ID, sorted most-recent-first.
pub fn load_reports(monitor_id: &str) -> Vec<MonitoringReport> {
    let dir = std::path::Path::new(REPORTS_DIR);
    if !dir.exists() {
        return Vec::new();
    }

    let prefix = format!("{}_", monitor_id);
    let mut reports: Vec<MonitoringReport> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".json") && !name.ends_with(".tmp")
        })
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| serde_json::from_str::<MonitoringReport>(&s).ok())
        .collect();

    // Most recent first.
    reports.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    reports
}

/// Load just the latest report for a given monitor ID.
pub fn load_latest_report(monitor_id: &str) -> Option<MonitoringReport> {
    load_reports(monitor_id).into_iter().next()
}

/// List all distinct monitor IDs that have at least one saved report.
pub fn list_monitor_ids() -> Vec<String> {
    let dir = std::path::Path::new(REPORTS_DIR);
    if !dir.exists() {
        return Vec::new();
    }

    let mut ids: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy().to_string();
            if !name.ends_with(".json") || name.ends_with(".tmp") {
                return None;
            }
            // filename format: <monitor_id>_<timestamp>.json
            // monitor_id itself is a slug so it never contains underscores at the end
            // We split on the LAST underscore to separate id from timestamp.
            let stem = name.trim_end_matches(".json");
            let last_under = stem.rfind('_')?;
            Some(stem[..last_under].to_string())
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    ids.sort();
    ids
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn sample_report(monitor_id: &str, timestamp: u64) -> MonitoringReport {
        MonitoringReport {
            monitor_id: monitor_id.to_string(),
            intent: format!("monitor {monitor_id}"),
            timestamp,
            previous_run_timestamp: None,
            has_changes: true,
            change_summary: "1 change: 1 modified".to_string(),
            changes: vec![],
            current_payload: json!({ "price": "$25" }),
            previous_payload: Some(json!({ "price": "$20" })),
        }
    }

    #[test]
    fn monitoring_report_serialises_and_deserialises() {
        let r = sample_report("test_monitor", 1_700_000_000);
        let json = serde_json::to_string(&r).unwrap();
        let back: MonitoringReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.monitor_id, "test_monitor");
        assert_eq!(back.timestamp, 1_700_000_000);
        assert!(back.has_changes);
    }

    #[test]
    fn empty_changes_skipped_in_serialisation() {
        let r = sample_report("m", 1);
        let json = serde_json::to_string(&r).unwrap();
        // changes is empty → field should be absent
        assert!(!json.contains("\"changes\""), "empty changes should be skipped");
    }

    #[test]
    fn save_and_load_roundtrip() {
        // Use a temp directory so we don't pollute the real monitoring_reports/.
        // We can't easily override REPORTS_DIR in a test, so we'll just call the
        // internal helpers directly on a known path.
        let report = sample_report("roundtrip_test", 9_999_999_999);

        // Skip if persistence is disabled or we're in a CI environment without
        // write access.  This test is marked as potentially skipped.
        let dir = Path::new(REPORTS_DIR);
        if std::fs::create_dir_all(dir).is_err() {
            return; // silently skip
        }

        save_report(&report).expect("save should succeed");

        let loaded = load_latest_report("roundtrip_test");
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.timestamp, 9_999_999_999);
        assert_eq!(loaded.monitor_id, "roundtrip_test");

        // Cleanup
        let filename = format!("{}_{}.json", report.monitor_id, report.timestamp);
        let _ = std::fs::remove_file(dir.join(filename));
    }
}
