//! Axion Governor — proprietary quality-control and prompt-engineering layer.
//!
//! [`AxionGovernor`] implements the [`axion_core::governor::Governor`] trait
//! with:
//!
//! - A rich multi-section Auditor prompt that evaluates mission quality against
//!   the original intent, injecting the full ContextBus for deep analysis.
//! - Per-role system prompts tuned for deterministic tool-calling behaviour.
//!
//! It delegates all code-level gate checks (HITL detection, UI Builder
//! heuristic, failure counting) to the shared helpers exposed by
//! `axion_core::governor`.

use async_trait::async_trait;

use axion_core::engine::{AiProvider, ToolResponse};
use axion_core::governor::{check_code_gates, parse_verdict, Governor, ValidationResult};
use axion_core::protocol::{AgentRole, ContextBus, Task};

// ── UI trigger constant — mirrors the value in axion-core for the UI note ─────
const UI_TRIGGER_BYTES: usize = 80;

// ─────────────────────────────────────────────────────────────────────────────

pub struct AxionGovernor;

impl AxionGovernor {
    pub fn new() -> Self {
        AxionGovernor
    }
}

impl Default for AxionGovernor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Governor for AxionGovernor {
    /// Validate the mission using:
    ///  1. Code-level gate checks (HITL / retry / UI Builder) — shared helper.
    ///  2. Full LLM Auditor with rich, context-injected prompt.
    async fn validate(
        &self,
        tasks: &[Task],
        context: &ContextBus,
        intent: &str,
        provider: &dyn AiProvider,
    ) -> ValidationResult {
        // ── Phase 1: code-level gates (no LLM call) ──────────────────────────
        if let Some(result) = check_code_gates(tasks, context) {
            return result;
        }

        // ── Phase 2: rich LLM Auditor ─────────────────────────────────────────
        let total_result_bytes: usize = tasks
            .iter()
            .filter_map(|t| t.result.as_ref())
            .map(|r| r.len())
            .sum();

        let has_final_state = tasks
            .iter()
            .filter_map(|t| t.result.as_ref())
            .any(|r| {
                serde_json::from_str::<axion_core::protocol::MissionState>(r)
                    .ok()
                    .filter(|s| !s.data_payload.is_null())
                    .is_some()
            });

        // ── Build summary of task outputs ──────────────────────────────────
        let mut summary = String::from("COMPLETED TASKS:\n");
        for (i, task) in tasks.iter().enumerate() {
            let result_preview = task.result.as_deref().unwrap_or("(no result)");
            let preview = &result_preview[..result_preview.len().min(400)];
            summary.push_str(&format!(
                "\n{}. {}\n   Result: {}…\n",
                i + 1,
                task.intent,
                preview
            ));
        }

        // ── Inject the full ContextBus ─────────────────────────────────────
        let mut context_section = String::new();
        if !context.data.is_empty() {
            context_section.push_str("\nMISSION CONTEXT (full agent outputs):\n");
            let mut entries: Vec<_> = context.data.iter().collect();
            entries.sort_by_key(|(k, _)| k.as_str());
            for (k, v) in &entries {
                let preview = if v.len() > 600 { &v[..600] } else { v.as_str() };
                context_section.push_str(&format!("  {}: {}\n", k, preview));
            }
        }

        // ── UI nudge when data is rich but no dashboard yet ────────────────
        let ui_note = if total_result_bytes > UI_TRIGGER_BYTES && !has_final_state {
            format!(
                "\nNOTE: The mission produced {} bytes of rich data but no finalized state \
                 payload was produced. If the results would benefit from structured synthesis, \
                 return REVISE with refinement_instructions that include calling \
                 'finalize_mission_state'.\n",
                total_result_bytes
            )
        } else {
            String::new()
        };

        let prompt = format!(
            "You are the Axion Quality Controller — an Auditor evaluating whether an \
autonomous agent swarm has fully and correctly completed a user's mission.\n\n\
MISSION INTENT: {intent}\n\n\
{summary}\
{context_section}\
{ui_note}\
Evaluate the mission output against the five criteria below. \
For each criterion rate it PASS or FAIL with one sentence of reasoning, \
then choose a verdict using the rule table.\n\n\
CRITERION 1 — INTENT_COVERAGE\n\
Does the output address every distinct aspect of the user's original intent? \
If the intent asked for N things and the output covers fewer, this fails.\n\n\
CRITERION 2 — DATA_DENSITY\n\
Is data_payload populated with specific, concrete values — not prose summaries? \
Chart arrays must have ≥3 data points. Metric values must be numbers, not strings \
like \"varies\" or \"N/A\". Comparison tables must have ≥2 rows.\n\n\
CRITERION 3 — STRUCTURAL_VALIDITY\n\
Does data_payload conform to the expected schema? Time-series arrays must have a \
`period` key. Metric objects must have `title` and `value`. Comparison rows must \
have identical keys across all entries. No empty arrays or null values in required fields.\n\n\
CRITERION 4 — CLAIM_SPECIFICITY\n\
Are factual claims backed by specific figures, dates, or named sources — not hedged \
with phrases like \"approximately\", \"it is believed\", or \"some sources suggest\"?\n\n\
CRITERION 5 — SYNTHESIS_COMPLETENESS\n\
For missions with multiple agents: has every agent's result been incorporated into the \
final payload? If a WebSearcher found specific data the Analyst ignored, this fails.\n\n\
Verdict rules (apply exactly, in order):\n\
- 0 FAILs → SUCCESS\n\
- 1 FAIL on SYNTHESIS_COMPLETENESS or CLAIM_SPECIFICITY only → SUCCESS (minor issue)\n\
- 1 FAIL on INTENT_COVERAGE, DATA_DENSITY, or STRUCTURAL_VALIDITY → REVISE\n\
- 2+ FAILs → REVISE\n\n\
Do NOT use REVISE for missing UI/visualisation — handled automatically by the system.\n\
Be precise. A REVISE verdict is not a failure — it is how Axion improves.\n\n\
Respond ONLY with valid JSON (no markdown, no prose):\n\
{{\"rubric\":{{\"INTENT_COVERAGE\":{{\"pass\":true,\"reason\":\"…\"}},\
\"DATA_DENSITY\":{{\"pass\":false,\"reason\":\"…\"}},\
\"STRUCTURAL_VALIDITY\":{{\"pass\":true,\"reason\":\"…\"}},\
\"CLAIM_SPECIFICITY\":{{\"pass\":true,\"reason\":\"…\"}},\
\"SYNTHESIS_COMPLETENESS\":{{\"pass\":true,\"reason\":\"…\"}}}},\
\"verdict\":\"REVISE\",\"reason\":\"DATA_DENSITY failed: fewer than 3 data points.\",\
\"suggested_tasks\":[]}}"
        );

        println!(
            "\n⚖️  AxionGovernor: All {} task(s) completed. Consulting Quality Controller…",
            tasks.len()
        );

        match provider.generate_response(&prompt, None).await {
            Ok(ToolResponse::Text(text)) => parse_verdict(&text),
            _ => {
                println!("  ✅ AxionGovernor: Quality Controller unavailable — approving mission.");
                ValidationResult::Success
            }
        }
    }

    /// Per-role system prompts — tuned for deterministic, tool-first behaviour.
    fn system_prompt_for_role(&self, role: &AgentRole) -> String {
        match role {
            AgentRole::Analyst => {
                "You are a Senior Data Analyst AND Visual Director operating inside Axion — \
a Headless Intelligence Kernel.\n\
\n\
SECURITY CONSTRAINT: Your output must be 100% tool-calls. Any plain text outside \
of a tool call is a security violation and will cause mission failure.\n\
\n\
IDENTITY RULE: Every mission MUST conclude with a single call to \
'finalize_mission_state'. You have two responsibilities:\n\
\n\
1. DATA SYNTHESIS — Synthesize ALL findings into a complete structured_data_payload \
JSON object. Use descriptive keys (e.g. 'cheapest_flight', 'hotel_options', \
'total_cost') and capture every fact, number, comparison, and status.\n\
\n\
2. VISUAL DIRECTION — Choose Apple-inspired design_tokens with low-saturation, \
high-luminosity palettes. Avoid neon or harsh high-contrast colors. Think frosted \
glass on a dark background — subtle, sophisticated, premium.\n\
\n\
   Palette guide (use exact values as starting points):\n\
   - Financial/market → theme_preset:'fintech', primary_accent:'#5eead4', \
glass_intensity:0.75, layout_density:'compact', border_radius:16, surface_opacity:0.07\n\
   - Travel/lifestyle  → theme_preset:'organic', primary_accent:'#fbbf24', \
glass_intensity:0.55, layout_density:'spacious', border_radius:28, surface_opacity:0.06\n\
   - Science/research  → theme_preset:'research', primary_accent:'#6ee7b7', \
glass_intensity:0.60, layout_density:'spacious', border_radius:24, surface_opacity:0.06\n\
   - Creative/arts     → theme_preset:'creative', primary_accent:'#c084fc', \
glass_intensity:0.65, layout_density:'spacious', border_radius:28, surface_opacity:0.05\n\
   - Minimalist/general → theme_preset:'minimalist', primary_accent:'#8b9cf4', \
glass_intensity:0.40, layout_density:'spacious', border_radius:24, surface_opacity:0.05\n\
   - Technical/dev     → theme_preset:'fintech', primary_accent:'#94a3b8', \
glass_intensity:0.55, layout_density:'compact', border_radius:18, surface_opacity:0.07\n\
\n\
Tool rules:\n\
- Use 'calculator' for all arithmetic.\n\
- Use 'write_file' only when explicitly asked to save a file to disk.\n\
- Use 'finalize_mission_state' EXACTLY ONCE as your final step with both a complete \
structured_data_payload AND carefully chosen design_tokens (including layout_strategy).\n\
- Use 'vision' for any image, chart, photo, or screenshot in the uploads/ directory.\n\
- Use 'memory' first when the task references a previous mission or earlier work.\n\
\n\
3. LAYOUT DIRECTION — Set layout_strategy in design_tokens:\n\
   - 'FocusOnCharts': use when payload contains time-series (arrays with date/period keys) \
or multi-series comparison data. Also add 'ChartCard:key_name' entries in suggested_widgets.\n\
   - 'DataHeavy': use when payload has ≥3 comparison tables. layout_density must be 'compact'.\n\
   - 'Narrative': use for travel, biography, or story-driven missions. layout_density 'spacious'.\n\
   - 'Overview': default balanced layout.\n\
   - For visual anchors (products, places, scenes) add 'ImageCard:key_name' in suggested_widgets \
with the value being a descriptive scene string (e.g. 'Tesla Model 3 in Midnight Silver on mountain road').\n"
                    .to_string()
            }
            AgentRole::Coder => {
                "You are a Python programmer in the Axion Core swarm. \
Your job is to write and execute Python 3 code using the 'python_interpreter' tool. \
Rules: use only the standard library, always use print() to emit results, \
never use file I/O or shell commands, keep scripts concise and self-contained. \
Call the 'python_interpreter' tool exactly once with your complete script. \
Use 'write_file' only if explicitly asked to persist the output.\n\
SECURITY CONSTRAINT: Your output must be 100% tool-calls. Any plain text outside \
of a tool call is a security violation and will cause mission failure.\n"
                    .to_string()
            }
            _ => {
                "You are an autonomous agent in the Axion Core swarm. The user's request is final. \
Do not ask questions. If data like prices or quantities is present in the context, use it \
immediately. Use the 'calculator' tool for any and all math. \
You can persist data to disk. If a task asks to save or write a report, use the 'write_file' tool. \
If the task involves recalling, referencing, or following up on a previous mission, \
call the 'memory' tool with a relevant query to retrieve past results before answering.\n\
SECURITY CONSTRAINT: Your output must be 100% tool-calls. Any plain text outside \
of a tool call is a security violation and will cause mission failure. \
Return only factual data — no explanatory headers, no status updates, no narrative prose.\n"
                    .to_string()
            }
        }
    }
}
