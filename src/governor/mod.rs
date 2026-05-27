use async_trait::async_trait;

use crate::engine::{AiProvider, ToolResponse};
use crate::protocol::{AgentRole, ContextBus, MissionState, Task, TaskStatus, CTX_OUTPUT_SCHEMA, CTX_USER_FEEDBACK};

// Total result text above this byte threshold (across ≥2 completed tasks) is
// considered "data-rich" and triggers a UI generation pass.
const UI_TRIGGER_BYTES: usize = 80;

// ── Public types ──────────────────────────────────────────────────────────────

/// The Governor's verdict after reviewing a completed mission round.
pub enum ValidationResult {
    /// All tasks are complete and quality-approved.
    Success,
    /// One or more tasks failed — reset and retry.
    Retry,
    /// Research reveals new requirements; expand the mission with these tasks.
    Expand(Vec<NewTask>),
    /// Results exist but quality is poor; inject refinement tasks to fix them.
    Refine(Vec<NewTask>),
    /// An agent used the `feedback` tool — pause and return control to the user.
    AwaitingFeedback {
        /// The question the agent wants the user to answer.
        question: String,
    },
}

/// A new task the Governor wants to append to the mission.
pub struct NewTask {
    pub description: String,
    pub role: AgentRole,
    /// Tools that must not be offered to this task's agent (e.g. after a
    /// tool failure the re-planner blacklists the failing tool so the next
    /// attempt is forced onto an alternative approach).
    pub excluded_tools: Vec<String>,
}

// ── Governor trait (public interface) ─────────────────────────────────────────

/// Quality-control and prompt-engineering interface.
///
/// Implement this trait to plug in a custom Governor (e.g.
/// [`opalzero_kernel::governor::OpalZeroGovernor`]).  The built-in open-source
/// implementation is [`BuiltinGovernor`].
#[async_trait]
pub trait Governor: Send + Sync {
    /// Review all completed tasks and decide the next action.
    ///
    /// `provider` is available for implementations that want to consult an LLM
    /// for quality assessment.
    async fn validate(
        &self,
        tasks: &[Task],
        context: &ContextBus,
        intent: &str,
        provider: &dyn AiProvider,
    ) -> ValidationResult;

    /// Return the system-prompt prefix for an agent executing in `role`.
    ///
    /// The dispatcher prepends this to every agent prompt so role-specific
    /// behaviour (tool preference, output style, constraints) can be tuned
    /// without modifying the core dispatch loop.
    fn system_prompt_for_role(&self, role: &AgentRole) -> String;

    /// Minimal Analyst prompt used when the caller supplies an explicit output
    /// schema. Omits all component-type examples so the schema contract wins.
    fn schema_analyst_prompt(&self) -> String {
        "You are the Analyst agent in the OpalZero multi-agent system.\n\
The WebSearcher agents have already collected all needed data — it is in PREVIOUS TASK RESULTS.\n\
Your ONLY job is to call finalize_mission_state ONCE with a structured data_payload.\n\
\n\
RULES:\n\
* Read the MANDATORY OUTPUT SCHEMA at the top — use EXACTLY those key names, no others.\n\
* Fill each key from the PREVIOUS TASK RESULTS already in your context.\n\
* Do NOT call web_search first — the WebSearchers already ran. Call finalize_mission_state directly.\n\
* Numbers must be numeric type — NEVER strings. 11000000000 not \"11B\".\n\
* Arrays must be real arrays, not comma-separated strings.\n\
* Omit a key only if you truly have zero data for it — never invent values.\n\
\n\
PARTIAL DATA: If some sections have no data, still call finalize_mission_state with\n\
whatever you have. Partial results are always better than nothing.\n\
\n\
⚠️  CRITICAL: Your FIRST and ONLY tool call must be finalize_mission_state. Never write prose.\n\
".to_string()
    }

    /// Optional: return the model string to use for a given agent role.
    ///
    /// The dispatcher calls this before every task and, when `Some(model)` is
    /// returned, creates a role-specific provider via
    /// [`AiProvider::with_text_model`].  Return `None` to use the provider's
    /// default model unchanged.
    fn model_for_role(&self, _role: &AgentRole) -> Option<String> {
        None
    }

    /// Optional: return the model string to use for the Governor's own quality
    /// checks inside [`validate`].
    ///
    /// When `Some(model)` is returned, the run loop in `lib.rs` creates a
    /// critic-specific provider via [`AiProvider::with_text_model`] and passes
    /// it to `validate()` instead of the plan's main provider.  This lets you
    /// route quality assessment to a faster / cheaper model (e.g. `gpt-4o-mini`)
    /// or to a completely different model class than the one used by agents.
    ///
    /// Return `None` (the default) to use the plan's provider unchanged.
    fn critic_model(&self) -> Option<String> {
        None
    }
}

// ── Shared code-level gate helpers ────────────────────────────────────────────

/// Run all pure-code gate checks that do not require an LLM call.
///
/// Returns `Some(result)` if a gate fired and the caller should return
/// immediately, or `None` if all gates passed and the caller should proceed to
/// its own quality-assessment step.
///
/// Checks performed (in order):
/// 1. **HITL** — any completed task result starts with
///    [`crate::protocol::AWAITING_FEEDBACK_PREFIX`] and the user has not yet
///    responded → [`ValidationResult::AwaitingFeedback`].
/// 2. **Failure / incomplete** — failed or stuck tasks → [`ValidationResult::Retry`].
/// 3. **UI Builder** — data-rich results but no [`UIBlueprint`] yet →
///    [`ValidationResult::Expand`] with a `build_dynamic_ui` task.
pub fn check_code_gates(tasks: &[Task], context: &ContextBus) -> Option<ValidationResult> {
    // ── 1. HITL ───────────────────────────────────────────────────────────────
    let prefix = crate::protocol::AWAITING_FEEDBACK_PREFIX;
    if !context.data.contains_key(CTX_USER_FEEDBACK) {
        for task in tasks.iter().filter(|t| matches!(t.status, TaskStatus::Completed)) {
            if let Some(ref result) = task.result {
                if let Some(question) = result.strip_prefix(prefix) {
                    tracing::info!(question, "Governor: mission paused — awaiting human feedback");
                    return Some(ValidationResult::AwaitingFeedback {
                        question: question.to_string(),
                    });
                }
            }
        }
    }

    // ── 2. Failure / incomplete ────────────────────────────────────────────────
    let failed_count = tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Failed))
        .count();
    let completed_count = tasks
        .iter()
        .filter(|t| matches!(t.status, TaskStatus::Completed))
        .count();
    let total = tasks.len();

    if failed_count > 0 {
        tracing::warn!(failed_count, "task(s) failed — marking for retry");
        return Some(ValidationResult::Retry);
    }

    if completed_count < total {
        tracing::info!(completed_count, total, "tasks complete — waiting for remaining");
        return Some(ValidationResult::Retry);
    }

    // ── 3. State Finalizer ────────────────────────────────────────────────────
    let has_final_state = tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .any(|r| {
            serde_json::from_str::<MissionState>(r)
                .ok()
                .filter(|s| !s.data_payload.is_null())
                .is_some()
        });

    let total_result_bytes: usize = tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .map(|r| r.len())
        .sum();

    // Block re-injection only while a finalize task is still in-flight.
    // A completed-but-prose task doesn't count as finalized, so the Governor
    // can inject a fresh attempt rather than silently approving empty output.
    let has_finalize_task = tasks.iter().any(|t| {
        matches!(t.role, AgentRole::Analyst)
            && t.intent.contains("finalize_mission_state")
            && matches!(t.status, TaskStatus::Pending | TaskStatus::Running)
    });

    if !has_final_state && !has_finalize_task && total_result_bytes >= UI_TRIGGER_BYTES && completed_count >= 2 {
        tracing::info!(total_result_bytes, completed_count, "Governor: State Finalizer triggered");

        // When the caller supplied an output schema, replace the generic Rome
        // travel example with one that uses the actual required keys so the LLM
        // doesn't copy the wrong structure.
        let schema_keys: Option<Vec<String>> = context
            .data
            .get(CTX_OUTPUT_SCHEMA)
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| {
                v.as_object()
                    .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            });

        let finalizer_description = if let Some(keys) = schema_keys {
            let keys_list = keys
                .iter()
                .map(|k| format!("      \"{k}\": <value from your research>"))
                .collect::<Vec<_>>()
                .join(",\n");
            let widget_examples = keys
                .iter()
                .take(3)
                .map(|k| format!("\"MetricCard:{k}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Call finalize_mission_state with a fully populated structured_data_payload.\n\
\n\
🔒 STRICT KEY CONTRACT — structured_data_payload MUST use ONLY these exact top-level keys:\n\
{keys_list_block}\
⛔ DO NOT add any other keys. Keys not in this list will be silently dropped and show NOTHING in the UI.\n\
⛔ Do NOT invent key names. Use the exact names listed above, verbatim.\n\
\n\
⚠️  COMMON MISTAKE — do NOT do this:\n\
  finalize_mission_state({{\n\
    \"summary\": \"Overview...\",\n\
    \"suggested_widgets\": [\"MetricCard:some_key\"],\n\
    \"verification_logs\": [...]\n\
  }})\n\
This is WRONG. suggested_widgets contains labels only — it does NOT contain the actual data.\n\
\n\
✅ CORRECT — structured_data_payload with ONLY the required keys:\n\
  finalize_mission_state({{\n\
    \"summary\": \"<one-sentence summary of findings>\",\n\
    \"suggested_widgets\": [{widget_examples}],\n\
    \"structured_data_payload\": {{\n\
{keys_list}\n\
    }}\n\
  }})\n\
\n\
Now extract ALL findings from the PREVIOUS TASK RESULTS in your context and call finalize_mission_state EXACTLY ONCE.",
                keys_list_block = keys
                    .iter()
                    .map(|k| format!("  - {k}\n"))
                    .collect::<String>(),
            )
        } else {
            "Call finalize_mission_state with a fully populated structured_data_payload.\n\
\n\
⚠️  COMMON MISTAKE — do NOT do this:\n\
  finalize_mission_state({\n\
    \"summary\": \"Overview...\",\n\
    \"suggested_widgets\": [\"MetricCard:cheapest_flight\", \"ComparisonTable:hotels\"],\n\
    \"verification_logs\": [...]\n\
  })\n\
This is WRONG. suggested_widgets contains labels only — it does NOT contain the actual data.\n\
\n\
✅ CORRECT — always include structured_data_payload with real values:\n\
  finalize_mission_state({\n\
    \"summary\": \"Weekend trip to Rome: flights from $24, hotels from $131/night.\",\n\
    \"suggested_widgets\": [\"MetricCard:cheapest_flight_usd\", \"ComparisonTable:hotels\", \"Timeline:itinerary\"],\n\
    \"structured_data_payload\": {\n\
      \"cheapest_flight_usd\": 24,\n\
      \"hotel_budget_min_usd\": 131,\n\
      \"hotels\": [\n\
        {\"name\": \"Hotel Vilon\", \"stars\": 5, \"price_per_night_usd\": 250},\n\
        {\"name\": \"Hotel Locarno\", \"stars\": 4, \"price_per_night_usd\": 180}\n\
      ],\n\
      \"sources\": [{\"label\": \"TripAdvisor\", \"url\": \"https://www.tripadvisor.com\"}]\n\
    }\n\
  })\n\
\n\
Now extract ALL findings from the PREVIOUS TASK RESULTS in your context and call finalize_mission_state EXACTLY ONCE with a structured_data_payload that matches this pattern."
                .to_string()
        };

        return Some(ValidationResult::Expand(vec![NewTask {
            description: finalizer_description,
            role: AgentRole::Analyst,
            excluded_tools: vec![],
        }]));
    }

    // ── 4. Conflict Verifier ──────────────────────────────────────────────────
    // When finalize_mission_state reported data_conflicts, auto-inject a
    // targeted WebSearcher to find the ground truth, then let the Governor
    // re-run finalize with the clarification appended to context.
    if has_final_state {
        let conflicts: Option<Vec<serde_json::Value>> = tasks
            .iter()
            .filter_map(|t| t.result.as_ref())
            .find_map(|r| {
                let state: MissionState = serde_json::from_str(r).ok()?;
                if let Some(arr) = state.data_payload.get("data_conflicts") {
                    if arr.is_array() && !arr.as_array().unwrap().is_empty() {
                        return arr.as_array().cloned();
                    }
                }
                None
            });

        if let Some(conflicts) = conflicts {
            // Only inject once — skip if a conflict-resolution task already exists.
            let already_resolving = tasks.iter().any(|t| {
                t.intent.contains("CONFLICT_RESOLVE")
            });

            if !already_resolving {
                let conflict_summary: Vec<String> = conflicts
                    .iter()
                    .take(3)
                    .filter_map(|c| {
                        let field = c["field"].as_str()?;
                        let vals: Vec<String> = c["values"]
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                        Some(format!("{}: {}", field, vals.join(" vs ")))
                    })
                    .collect();

                tracing::info!(conflict_count = conflicts.len(), "Governor: Conflict Verifier triggered — injecting resolution task");

                let query = format!(
                    "CONFLICT_RESOLVE — Verify these conflicting data points with an authoritative \
                     web search and return the correct value for each: {}. \
                     Append your findings to context so the Analyst can update the mission state.",
                    conflict_summary.join("; ")
                );

                return Some(ValidationResult::Expand(vec![NewTask {
                    description: query,
                    role: AgentRole::WebSearcher,
                    excluded_tools: vec![],
                }]));
            }
        }
    }

    tracing::debug!(
        total_result_bytes,
        threshold = UI_TRIGGER_BYTES,
        completed_count,
        "Governor: skipping State Finalizer",
    );

    None
}

// ── JSON parsing helpers (shared with opalzero-kernel) ───────────────────────────

/// Parse a Governor verdict from the raw LLM response text.
///
/// Handles the structured five-rubric JSON format (primary) and falls back
/// to plain-text keyword matching for backwards compatibility.
///
/// Exported so `opalzero-kernel`'s `OpalZeroGovernor` can reuse the same parsing
/// logic without duplicating it.
pub fn parse_verdict(response: &str) -> ValidationResult {
    // ── Structured rubric JSON (primary path) ─────────────────────────────────
    if let Some(json_str) = extract_json(response) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let verdict = val["verdict"].as_str().unwrap_or("").to_uppercase();
            let reason  = val["reason"].as_str().unwrap_or("").to_string();

            // Log every failing criterion for observability.
            if let Some(rubric) = val["rubric"].as_object() {
                for (criterion, result) in rubric {
                    let pass   = result["pass"].as_bool().unwrap_or(true);
                    let reason = result["reason"].as_str().unwrap_or("");
                    if !pass {
                        tracing::warn!(criterion, reason, "Governor: rubric criterion failed");
                    }
                }
            }

            return match verdict.as_str() {
                "SUCCESS" => {
                    tracing::info!("Governor: rubric passed — SUCCESS");
                    ValidationResult::Success
                }
                "REVISE" => {
                    // Build expansion tasks from suggested_tasks if the LLM provided any.
                    let new_tasks: Vec<NewTask> = val["suggested_tasks"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                               .filter_map(|t| {
                                   // Accept both "intent" and "description" keys from the JSON.
                                   let desc = t["intent"].as_str()
                                       .or_else(|| t["description"].as_str())?;
                                   let role = match t["role"].as_str().unwrap_or("") {
                                       "Analyst" => AgentRole::Analyst,
                                       "Coder"   => AgentRole::Coder,
                                       _         => AgentRole::WebSearcher,
                                   };
                                   Some(NewTask {
                                       description:    desc.to_string(),
                                       role,
                                       excluded_tools: vec![],
                                   })
                               })
                               .collect()
                        })
                        .unwrap_or_default();

                    if new_tasks.is_empty() {
                        tracing::warn!(reason, "Governor: REVISE — retrying");
                        ValidationResult::Retry
                    } else {
                        tracing::info!(new_task_count = new_tasks.len(), reason, "Governor: EXPAND");
                        ValidationResult::Expand(new_tasks)
                    }
                }
                _ => {
                    tracing::warn!(verdict, "Governor: unrecognised verdict — defaulting to Success");
                    ValidationResult::Success
                }
            };
        }
    }

    // ── Plain-text fallback (backwards compatibility) ─────────────────────────
    let upper = response.to_uppercase();
    if upper.contains("SUCCESS") { return ValidationResult::Success; }
    if upper.contains("EXPAND")  { return ValidationResult::Expand(vec![]); }
    if upper.contains("REVISE") || upper.contains("RETRY") {
        return ValidationResult::Retry;
    }
    ValidationResult::Success
}

/// Find and return the first well-formed JSON object in `text`.
///
/// Scans forward character by character, attempts a structural parse at each
/// `{`, and returns the first candidate that `serde_json` validates.  This
/// correctly handles:
/// - Nested objects: `{ "payload": { "a": 1 } }`
/// - Strings containing braces: `{ "text": "result is {ok}" }`
/// - Multiple JSON objects in a single response (returns the first valid one)
///
/// Returns `None` when no valid JSON object is found (e.g. the LLM returned
/// plain prose).
pub fn extract_json(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if bytes[start] != b'{' {
            continue;
        }
        let slice = &text[start..];
        // Walk forward tracking brace depth to find candidate end positions.
        let mut depth:  i32  = 0;
        let mut in_str: bool = false;
        let mut escape: bool = false;
        for (i, ch) in slice.char_indices() {
            if escape           { escape = false; continue; }
            if ch == '\\'       { escape = true;  continue; }
            if ch == '"'        { in_str = !in_str; continue; }
            if in_str           { continue; }
            if ch == '{'        { depth += 1; }
            if ch == '}'        { depth -= 1; }
            if depth == 0 && i > 0 {
                let candidate = &slice[..=i];
                if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                    return Some(candidate.to_string());
                }
                // This start position produced a balanced but invalid object —
                // advance to the next `{` and try again.
                break;
            }
        }
    }
    None
}

// ── BuiltinGovernor ───────────────────────────────────────────────────────────

/// Default open-source Governor implementation.
///
/// Runs all code-level gate checks (HITL, retry, UI Builder) and falls back to
/// a lightweight LLM quality assessment using a generic prompt.  No proprietary
/// Auditor prompts are included.
///
/// For the full-quality Auditor, use
/// [`opalzero_kernel::governor::OpalZeroGovernor`] in the `opalzero-kernel` crate.
pub struct BuiltinGovernor {
    /// Model to use for quality-check LLM calls.  `None` → same provider as agents.
    /// Set via `OPALZERO_CRITIC_MODEL` env var or the builder method.
    critic_model: Option<String>,
}

impl BuiltinGovernor {
    pub fn new() -> Self {
        BuiltinGovernor {
            critic_model: std::env::var("OPALZERO_CRITIC_MODEL")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    /// Override the critic model at build time.
    pub fn with_critic_model(mut self, model: impl Into<String>) -> Self {
        self.critic_model = Some(model.into());
        self
    }
}

impl Default for BuiltinGovernor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Governor for BuiltinGovernor {
    async fn validate(
        &self,
        tasks: &[Task],
        context: &ContextBus,
        intent: &str,
        provider: &dyn AiProvider,
    ) -> ValidationResult {
        // Run code-level gates first (HITL / retry / UI Builder).
        if let Some(result) = check_code_gates(tasks, context) {
            return result;
        }

        // ── Requirement Tracker: verify all sub-requirements are addressed ────
        if let Some(req_json) = context.data.get(crate::safety::CTX_REQUIREMENTS) {
            let requirements = crate::safety::deserialize_requirements(req_json);
            if !requirements.is_empty() {
                // Collect the results of all completed tasks into a single string.
                let task_results: String = tasks
                    .iter()
                    .filter_map(|t| t.result.as_ref())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n---\n");

                let (satisfied, total, unmet) =
                    crate::safety::check_requirements(&requirements, &task_results, intent, provider)
                        .await;

                if total > 0 && !unmet.is_empty() {
                    let unmet_str = unmet.join("; ");
                    tracing::warn!(
                        satisfied,
                        total,
                        unmet = %unmet_str,
                        "RequirementTracker: unmet requirements — requesting retry"
                    );
                    return ValidationResult::Retry;
                }
            }
        }

        // Generic LLM quality check.
        tracing::info!(task_count = tasks.len(), "BuiltinGovernor: all tasks completed — consulting Quality Controller");

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

        let prompt = format!(
            "You are a mission quality reviewer for an autonomous AI agent swarm.\n\
Review the completed tasks for mission: {intent}\n\n\
{summary}\n\n\
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
For missions with multiple agents: has every agent's result been incorporated into \
the final payload? If a WebSearcher found specific data the Analyst ignored, this fails.\n\n\
Verdict rules (apply exactly, in order):\n\
- 0 FAILs → SUCCESS\n\
- 1 FAIL on SYNTHESIS_COMPLETENESS or CLAIM_SPECIFICITY only → SUCCESS (minor issue)\n\
- 1 FAIL on INTENT_COVERAGE, DATA_DENSITY, or STRUCTURAL_VALIDITY → REVISE\n\
- 2+ FAILs → REVISE\n\n\
Do NOT use REVISE for missing UI/visualisation — handled automatically by the system.\n\
Be precise. A REVISE verdict is not a failure — it is how OpalZero improves.\n\n\
Respond ONLY with valid JSON (no markdown, no prose):\n\
{{\"rubric\":{{\"INTENT_COVERAGE\":{{\"pass\":true,\"reason\":\"…\"}},\
\"DATA_DENSITY\":{{\"pass\":false,\"reason\":\"…\"}},\
\"STRUCTURAL_VALIDITY\":{{\"pass\":true,\"reason\":\"…\"}},\
\"CLAIM_SPECIFICITY\":{{\"pass\":true,\"reason\":\"…\"}},\
\"SYNTHESIS_COMPLETENESS\":{{\"pass\":true,\"reason\":\"…\"}}}},\
\"verdict\":\"REVISE\",\"reason\":\"DATA_DENSITY failed: fewer than 3 data points.\",\
\"suggested_tasks\":[]}}"
        );

        match provider.generate_response(&prompt, None).await {
            Ok(ToolResponse::Text(text)) => parse_verdict(&text),
            _ => {
                tracing::warn!("BuiltinGovernor: Quality Controller unavailable — approving mission");
                ValidationResult::Success
            }
        }
    }

    fn critic_model(&self) -> Option<String> {
        self.critic_model.clone()
    }

    fn system_prompt_for_role(&self, role: &AgentRole) -> String {
        let base = match role {
            AgentRole::Analyst => {
                "You are an Analyst agent. Your ONLY output is a single call to 'finalize_mission_state'. Never write prose. Never skip the call.\n\
\n\
⚠️  CRITICAL: If you call finalize_mission_state WITHOUT a populated structured_data_payload, the mission will show nothing to the user. Always fill it.\n\
\n\
TOOLS (use as needed before the final call):\n\
- 'calculator': arithmetic.\n\
- 'write_file': save Markdown to disk.\n\
- 'vision': analyze images.\n\
- 'feedback': request human input.\n\
\n\
═══ PAYLOAD SCHEMA RULES ═══\n\
\n\
1. TIME-SERIES & CHARTS — use an array of objects. REQUIRED keys: \"period\" (string label) and at least one numeric key. Numbers MUST be number type, never strings.\n\
   CORRECT: [{\"period\":\"Jan\",\"value\":42000},{\"period\":\"Feb\",\"value\":47000}]\n\
   WRONG:   [{\"period\":\"Jan\",\"value\":\"42000\"}]  ← strings kill the chart\n\
   WRONG:   {\"jan\":42000,\"feb\":47000}              ← flat objects become tables\n\
\n\
2. COMPARATIVE TABLES — array of objects with identical keys per row:\n\
   [{\"name\":\"Option A\",\"price\":120,\"rating\":4.5},{\"name\":\"Option B\",\"price\":95,\"rating\":4.2}]\n\
\n\
3. SINGLE METRICS — object with exactly 'title' and 'value' keys (plus optional 'unit', 'trend', 'subtitle'):\n\
   {\"title\":\"Market Cap\",\"value\":\"$2.8T\",\"trend\":\"up\"}\n\
\n\
4. STATUS — object with 'label' and 'status' keys. status must be one of: success/warning/error/info.\n\
\n\
5. VISUAL SCENES — string value under a key whose name contains 'image', 'visual', 'scene', or 'photo':\n\
   \"destination_scene\": \"Moonlit cobblestone streets of Rome's Trastevere district at dusk\"\n\
\n\
6. SOURCES — always include: \"sources\": [{\"label\":\"Site Name\",\"url\":\"https://...\"}]\n\
\n\
7. CONFLICTS — if two sources report contradictory values for the same metric, include:\n\
   \"data_conflicts\": [{\"field\":\"metric name\",\"values\":[\"38%\",\"59%\"],\"sources\":[\"Source A\",\"Source B\"]}]\n\
\n\
═══ suggested_widgets (include when applicable) ═══\n\
Hint to the frontend which component type to use for each payload key:\n\
- 'ChartCard:key_name'  for every time-series or comparative numeric array\n\
- 'ImageCard:key_name'  for every visual/scene string\n\
- Example: [\"ChartCard:revenue_trend\",\"ChartCard:market_share\",\"ImageCard:destination_scene\"]\n"
                    .to_string()
            }
            AgentRole::Coder => {
                "You are a Coder agent. Write Python 3 code using the 'python_interpreter' tool. \
Use only the standard library and print() for output.\n"
                    .to_string()
            }
            AgentRole::WebSearcher => {
                "You are a WebSearcher agent.\n\
- Use 'web_search' to find information.\n\
- In your result, always preserve source URLs exactly as they appear after 'Source:' in search results.\n\
- Structure your output as: FINDINGS: <data>, SOURCES: <url1>, <url2>, ... so the Analyst can extract them.\n\
- For entities with visual presence (places, products, people, companies), note them explicitly for the Analyst to create ImageCard entries.\n"
                    .to_string()
            }
            AgentRole::Planner => {
                "You are a Planner agent. Research and plan using available tools.\n".to_string()
            }
        };
        // Append the grounding rule to every role so factual claims are always
        // sourced from tool results, not reconstructed from the model's memory.
        format!("{}{}", base, crate::safety::grounding_suffix())
    }
}

// ── reset_failed_tasks ────────────────────────────────────────────────────────

/// Reset all `Failed` tasks to `Pending` so the Dispatcher can retry them.
pub fn reset_failed_tasks(tasks: &mut Vec<Task>) {
    for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Failed)) {
        task.status = TaskStatus::Pending;
    }
}
