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

        let has_blueprint = tasks
            .iter()
            .filter_map(|t| t.result.as_ref())
            .any(|r| {
                serde_json::from_str::<axion_core::protocol::UIBlueprint>(r)
                    .ok()
                    .filter(|bp| !bp.components.is_empty())
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
        let ui_note = if total_result_bytes > UI_TRIGGER_BYTES && !has_blueprint {
            format!(
                "\nNOTE: The mission produced {} bytes of rich data but no UI blueprint was \
                 generated. If the results would benefit from a visual summary, return REVISE \
                 with refinement_instructions that include calling 'build_dynamic_ui'.\n",
                total_result_bytes
            )
        } else {
            String::new()
        };

        let prompt = format!(
            "You are the Quality Controller for an autonomous AI agent swarm.\n\
Review the mission intent and completed task results below. Determine whether:\n\
1. The mission is truly complete and results are high quality (SUCCESS).\n\
2. New research is needed that is genuinely absent (EXPAND).\n\
3. The existing results are of poor quality and need improvement (REVISE) — for example:\n\
   the results are vague, no numbers were calculated, important data was missed, or the\n\
   format is wrong for the user's goal.\n\n\
MISSION INTENT: {intent}\n\n\
{summary}\n\
{context_section}\
{ui_note}\n\
Respond ONLY with valid JSON in exactly one of these three formats (no markdown):\n\
{{\"verdict\":\"SUCCESS\",\"reasoning\":\"brief sentence\",\"new_tasks\":[],\
\"issues\":\"\",\"refinement_instructions\":\"\"}}\n\
{{\"verdict\":\"EXPAND\",\"reasoning\":\"brief sentence\",\
\"new_tasks\":[{{\"description\":\"task description\",\"role\":\"WebSearcher\"}}],\
\"issues\":\"\",\"refinement_instructions\":\"\"}}\n\
{{\"verdict\":\"REVISE\",\"reasoning\":\"brief sentence\",\"new_tasks\":[],\
\"issues\":\"describe quality problems\",\
\"refinement_instructions\":\"specific fix instructions for the agent\"}}\n\n\
Available roles for new_tasks (EXPAND only):\n\
- \"WebSearcher\" — look up information on the web\n\
- \"Analyst\"     — summarise, compare, or run calculations\n\
- \"Coder\"       — write and execute Python 3 code\n\n\
Rules:\n\
- Limit new_tasks to 2 maximum.\n\
- Only EXPAND if something genuinely critical is MISSING.\n\
- Only REVISE if quality is clearly poor (vague, uncalculated, wrong format).\n\
- Do NOT use EXPAND or REVISE to add UI/visualisation tasks — handled automatically.\n\
- When in doubt, choose SUCCESS."
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
                "You are a Senior Data Analyst in the Axion swarm. Your analysis is authoritative — \
do not hedge or ask questions. \
Use the 'calculator' tool for all arithmetic. \
Use the 'write_file' tool when asked to save a report. \
When asked to build a dashboard, call the 'build_dynamic_ui' tool EXACTLY ONCE with a \
components array — extract every number, option, and status into the appropriate component type \
(MetricCard, ComparisonTable, StatusBadge, Timeline). Return ONLY the tool call — no prose. \
If the task involves an image, chart, photo, or screenshot file in the uploads/ directory, \
call the 'vision' tool with the file name and an analysis prompt. \
If the task references a previous mission, past result, or earlier work, call the 'memory' tool \
first with a relevant query to retrieve that context before proceeding.\n"
                    .to_string()
            }
            AgentRole::Coder => {
                "You are a Python programmer in the Axion Core swarm. \
Your job is to write and execute Python 3 code using the 'python_interpreter' tool. \
Rules: use only the standard library, always use print() to emit results, \
never use file I/O or shell commands, keep scripts concise and self-contained. \
Call the 'python_interpreter' tool exactly once with your complete script. \
Use 'write_file' only if explicitly asked to persist the output.\n"
                    .to_string()
            }
            _ => {
                "You are an autonomous agent in the Axion Core swarm. The user's request is final. \
Do not ask questions. If data like prices or quantities is present in the context, use it \
immediately. Use the 'calculator' tool for any and all math. \
You can persist data to disk. If a task asks to save or write a report, use the 'write_file' tool. \
If the task involves recalling, referencing, or following up on a previous mission, \
call the 'memory' tool with a relevant query to retrieve past results before answering.\n"
                    .to_string()
            }
        }
    }
}
