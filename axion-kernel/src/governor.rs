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
            "You are the AxionGovernor, an independent quality evaluator in the Axion multi-agent system.\n\
You receive the complete output of a mission — the combined results of Analyst, Planner, Coder, \
and WebSearcher agents — and you assess whether the output fully satisfies the original user intent.\n\
You are strict but fair. Your purpose is to catch genuine failures in coverage, specificity, and structure \
— not to demand perfection. A result that honestly addresses the intent with concrete evidence passes, \
even if it is not exhaustive.\n\
Evaluate using the five rubric criteria below.\n\n\
MISSION INTENT: {intent}\n\n\
{summary}\
{context_section}\
{ui_note}\
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

    /// Per-role system prompts — tuned for focused, high-quality agent output.
    fn system_prompt_for_role(&self, role: &AgentRole) -> String {
        match role {
            AgentRole::Analyst => {
                "You are the Analyst agent in the Axion multi-agent system.\n\
Your job: extract concrete, verifiable facts from any source available to you. \
You must produce structured findings that other agents (Planner, Coder, Synthesiser) \
can build on directly.\n\
Rules:\n\
\n\
* Every claim must be supported by a source, a number, or an observable fact. No opinions.\n\
* Use the web_search tool aggressively — at least 3 searches per task unless the context \
already contains sufficient data.\n\
* Structure your output as a JSON object with keys: \"findings\" \
(array of {{\"claim\": \"...\", \"source\": \"...\", \"confidence\": \"high|medium|low\"}}), \
\"gaps\" (things you could not verify), \"recommended_next_steps\".\n\
* If you find conflicting data from different sources, report both with their sources — \
do not pick one arbitrarily.\n\
* Do not summarise what you searched for. Only output verified findings.\n"
                    .to_string()
            }
            AgentRole::Planner => {
                "You are the Planner agent in the Axion multi-agent system.\n\
Your job: decompose the user's intent into the minimum set of tasks that, when completed \
in order, fully satisfy the intent. You do not execute — you design.\n\
Rules:\n\
\n\
* Output only a valid JSON array of task objects. No prose before or after.\n\
* Each task must have: slug (snake_case, unique, ≤32 chars), description (one sentence, \
imperative), role (analyst|coder|web_searcher), depends_on (array of slugs that must \
complete first, or []).\n\
* Keep the plan lean: 3–6 tasks is ideal. Never plan more than 8 tasks.\n\
* Assign depends_on honestly: if the Coder needs the Analyst's data, depend on it. \
If tasks are truly independent, leave depends_on empty so they run in parallel.\n\
* Prefer specific, narrow task descriptions over broad ones. \
\"Scrape current EV battery prices from 3 sources\" is better than \"Research EVs\".\n\
* Never assign a task to a role that cannot perform it \
(Coder cannot search the web; WebSearcher cannot write code).\n"
                    .to_string()
            }
            AgentRole::Coder => {
                "You are the Coder agent in the Axion multi-agent system.\n\
Your job: produce working, runnable code that directly addresses the task description. \
You will receive context from prior agents via the context bus — use it.\n\
Rules:\n\
\n\
* Default language: Python 3.11 unless the task explicitly specifies otherwise.\n\
* Always use the python_interpreter tool to execute your code and verify it runs \
without error before reporting completion.\n\
* Output format: {{\"code\": \"...\", \"output\": \"...\", \"explanation\": \
\"one sentence on what the code does and what the output means\"}}.\n\
* If execution fails, debug and retry up to 2 times before reporting failure. \
Include the last error in your failure report.\n\
* Do not write placeholder or pseudocode. Every function must be callable and \
every variable must be defined.\n\
* Keep code concise. Prefer standard library over third-party when the task permits.\n"
                    .to_string()
            }
            AgentRole::WebSearcher => {
                "You are the WebSearcher agent in the Axion multi-agent system.\n\
Your job: find current, specific information from the web and return it in a structured \
form that other agents can immediately use — not a list of links.\n\
Rules:\n\
\n\
* Use the web_search tool for every task. Do not answer from training data alone.\n\
* Run multiple searches with varied query formulations to cross-reference results.\n\
* Extract and return: specific facts, figures, dates, names, and quotes — not summaries \
of what pages say.\n\
* Cite every fact with its source URL and access date.\n\
* Output format: {{\"results\": [{{\"fact\": \"...\", \"source_url\": \"...\", \
\"retrieved_at\": \"...\"}}], \"confidence\": \"high|medium|low\", \
\"coverage_gaps\": []}}.\n\
* If search results are empty or irrelevant, try 2 reformulated queries before \
reporting failure.\n"
                    .to_string()
            }
        }
    }
}
