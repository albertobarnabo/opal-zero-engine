/// Mission observability: structured performance and quality metrics.
///
/// `MissionMetrics` is emitted at the end of every mission — both as a
/// structured `tracing::info!` event (for log aggregators) and as a
/// `MissionUpdate::Metrics` channel message (for the server / SSE stream).
///
/// Consumers can use these to track:
/// - Latency: `total_duration_ms`, per-task `task_timings`
/// - Cost proxy: `estimated_tokens` (chars ÷ 4 approximation)
/// - Research efficiency: `manifest_hits` vs total WebSearcher tasks
/// - Quality signal: `governor_verdicts` (retries, expansions, refinements)
use serde::{Deserialize, Serialize};

// ── Task-level timing ─────────────────────────────────────────────────────────

/// Wall-clock duration for a single dispatched task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskTiming {
    pub slug: String,
    pub role: String,
    /// How long the task ran (from dispatch to completion/failure), in ms.
    pub duration_ms: u64,
    /// Research manifest that short-circuited this task's execution, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_id: Option<String>,
}

// ── Governor verdict distribution ────────────────────────────────────────────

/// How many times each Governor verdict type was issued during the mission.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GovernorVerdicts {
    /// Times the Governor said "try again" and reset failed tasks.
    pub retries: u8,
    /// Times the Governor injected additional research tasks.
    pub expansions: u8,
    /// Times the Governor injected quality-refinement tasks.
    pub refinements: u8,
    /// Times the re-planner synthesised alternative tasks after repeated failures.
    pub repairs: u8,
}

// ── Mission-level metrics ─────────────────────────────────────────────────────

/// Structured performance and quality metrics emitted at the end of every mission.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissionMetrics {
    /// Wall-clock duration of the entire mission in milliseconds.
    pub total_duration_ms: u64,

    /// Total tasks created across all planning rounds
    /// (original plan + repair + expansion tasks).
    pub tasks_total: usize,
    /// Tasks that completed successfully.
    pub tasks_completed: usize,
    /// Tasks that failed after exhausting retries.
    pub tasks_failed: usize,

    /// WebSearcher tasks that resolved via a research manifest
    /// (bypassed the multi-turn Exa loop entirely).
    pub manifest_hits: usize,
    /// IDs of research manifests triggered during this mission (deduplicated).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub manifest_ids: Vec<String>,

    /// Governor verdict distribution for this mission.
    pub governor_verdicts: GovernorVerdicts,

    /// Per-task execution wall-clock durations.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub task_timings: Vec<TaskTiming>,

    /// Rough token estimate: sum of all task result character counts ÷ 4.
    /// Informational only — actual billed tokens vary by provider and model.
    pub estimated_tokens: u64,
}

impl MissionMetrics {
    /// Emit all fields as a single structured `tracing::info!` event.
    ///
    /// Log aggregators that parse structured tracing (e.g. Datadog, Grafana Loki
    /// with JSON format) will receive every field as a searchable attribute.
    pub fn log_structured(&self) {
        tracing::info!(
            total_duration_ms    = self.total_duration_ms,
            tasks_total          = self.tasks_total,
            tasks_completed      = self.tasks_completed,
            tasks_failed         = self.tasks_failed,
            manifest_hits        = self.manifest_hits,
            governor_retries     = self.governor_verdicts.retries,
            governor_expansions  = self.governor_verdicts.expansions,
            governor_refinements = self.governor_verdicts.refinements,
            governor_repairs     = self.governor_verdicts.repairs,
            estimated_tokens     = self.estimated_tokens,
            "mission_metrics",
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_serializes_to_json() {
        let m = MissionMetrics {
            total_duration_ms: 1234,
            tasks_total: 3,
            tasks_completed: 2,
            tasks_failed: 1,
            manifest_hits: 1,
            manifest_ids: vec!["company_intelligence".to_string()],
            governor_verdicts: GovernorVerdicts { retries: 1, expansions: 0, refinements: 0, repairs: 0 },
            task_timings: vec![
                TaskTiming { slug: "search_task".to_string(), role: "WebSearcher".to_string(), duration_ms: 800, manifest_id: Some("company_intelligence".to_string()) },
                TaskTiming { slug: "analyst_task".to_string(), role: "Analyst".to_string(), duration_ms: 434, manifest_id: None },
            ],
            estimated_tokens: 512,
        };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("total_duration_ms"));
        assert!(json.contains("manifest_hits"));
        assert!(json.contains("company_intelligence"));
        // manifest_id: None should be skipped
        assert!(!json.contains("null"));
    }

    #[test]
    fn empty_vecs_are_skipped_in_serialization() {
        let m = MissionMetrics::default();
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("manifest_ids"));
        assert!(!json.contains("task_timings"));
    }
}
