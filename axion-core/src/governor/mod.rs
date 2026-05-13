use async_trait::async_trait;
use serde::Deserialize;

use crate::engine::{AiProvider, ToolResponse};
use crate::protocol::{AgentRole, ContextBus, MissionState, Task, TaskStatus};

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
/// [`axion_kernel::governor::AxionGovernor`]).  The built-in open-source
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
    if !context.data.contains_key("user_feedback") {
        for task in tasks.iter().filter(|t| matches!(t.status, TaskStatus::Completed)) {
            if let Some(ref result) = task.result {
                if let Some(question) = result.strip_prefix(prefix) {
                    println!("  ⏸️  Governor: Mission paused — awaiting human feedback.");
                    println!("     Question: {}", question);
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
        println!("  ⚠️  {} task(s) failed. Marking for retry.", failed_count);
        return Some(ValidationResult::Retry);
    }

    if completed_count < total {
        println!(
            "  ⏳ {}/{} tasks complete — waiting for remaining.",
            completed_count, total
        );
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
        println!(
            "  🧠 Governor: State Finalizer triggered — {} bytes across {} task(s).",
            total_result_bytes, completed_count
        );
        return Some(ValidationResult::Expand(vec![NewTask {
            description:
                "Call finalize_mission_state. Extract ALL findings from the PREVIOUS TASK \
                 RESULTS in your context. Build a structured_data_payload JSON object where \
                 each key is a descriptive label (e.g. 'cheapest_flight', 'hotel_options', \
                 'total_cost') and each value captures the corresponding fact. \
                 Call finalize_mission_state EXACTLY ONCE with this complete payload."
                    .to_string(),
            role: AgentRole::Analyst,
            excluded_tools: vec![],
        }]));
    }

    println!(
        "  ⏭️  Governor: Skipping State Finalizer — {} bytes (threshold: {} bytes, {} task(s) completed).",
        total_result_bytes, UI_TRIGGER_BYTES, completed_count
    );

    None
}

// ── JSON parsing helpers (shared with axion-kernel) ───────────────────────────

#[derive(Deserialize)]
struct VerdictJson {
    verdict: String,
    #[serde(default)]
    new_tasks: Vec<NewTaskJson>,
    #[serde(default)]
    issues: String,
    #[serde(default)]
    refinement_instructions: String,
}

#[derive(Deserialize)]
struct NewTaskJson {
    description: String,
    #[serde(default)]
    role: String,
}

/// Parse a Governor verdict from the raw LLM response text.
///
/// Exported so `axion-kernel`'s `AxionGovernor` can reuse the same parsing
/// logic without duplicating it.
pub fn parse_verdict(response: &str) -> ValidationResult {
    let json = extract_json(response);
    match serde_json::from_str::<VerdictJson>(&json) {
        Ok(v) if v.verdict == "EXPAND" && !v.new_tasks.is_empty() => {
            let tasks: Vec<NewTask> = v
                .new_tasks
                .into_iter()
                .map(|t| NewTask {
                    description: t.description,
                    role: match t.role.as_str() {
                        "Analyst"  => AgentRole::Analyst,
                        "Planner"  => AgentRole::Planner,
                        "Coder"    => AgentRole::Coder,
                        _          => AgentRole::WebSearcher,
                    },
                    excluded_tools: vec![],
                })
                .collect();
            println!("  🔍 Governor: Expanding mission with {} new task(s).", tasks.len());
            for t in &tasks {
                println!("  ➕ New task [{}]: {}", t.role.as_str(), t.description);
            }
            ValidationResult::Expand(tasks)
        }
        Ok(v) if v.verdict == "REVISE" => {
            let description = if v.refinement_instructions.is_empty() {
                format!("Synthesize and fix the following quality issues: {}", v.issues)
            } else {
                format!(
                    "Synthesize and fix: {}. Instructions: {}",
                    v.issues, v.refinement_instructions
                )
            };
            println!("  🔧 Governor: Quality issues detected — requesting refinement.");
            if !v.issues.is_empty() {
                println!("     Issues: {}", v.issues);
            }
            ValidationResult::Refine(vec![NewTask {
                description,
                role: AgentRole::Analyst,
                excluded_tools: vec![],
            }])
        }
        Ok(_) => {
            println!("  ✅ Governor: Mission quality approved (SUCCESS).");
            ValidationResult::Success
        }
        Err(e) => {
            println!(
                "  ⚠️  Governor: Could not parse verdict ('{}') — defaulting to SUCCESS.",
                e
            );
            ValidationResult::Success
        }
    }
}

/// Strip markdown code fences and extract the first JSON object in the response.
pub fn extract_json(text: &str) -> String {
    let stripped = text.trim();
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}')) {
        return stripped[start..=end].to_string();
    }
    stripped.to_string()
}

// ── BuiltinGovernor ───────────────────────────────────────────────────────────

/// Default open-source Governor implementation.
///
/// Runs all code-level gate checks (HITL, retry, UI Builder) and falls back to
/// a lightweight LLM quality assessment using a generic prompt.  No proprietary
/// Auditor prompts are included.
///
/// For the full-quality Auditor, use
/// [`axion_kernel::governor::AxionGovernor`] in the `axion-kernel` crate.
pub struct BuiltinGovernor;

impl BuiltinGovernor {
    pub fn new() -> Self {
        BuiltinGovernor
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

        // Generic LLM quality check.
        println!(
            "\n⚖️  BuiltinGovernor: All {} task(s) completed. Consulting Quality Controller…",
            tasks.len()
        );

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
            "You are a mission quality reviewer.\n\
Review the completed tasks for mission: {intent}\n\n\
{summary}\n\n\
Respond ONLY with valid JSON (no markdown):\n\
{{\"verdict\":\"SUCCESS\",\"reasoning\":\"...\",\"new_tasks\":[],\
\"issues\":\"\",\"refinement_instructions\":\"\"}}\n\
{{\"verdict\":\"EXPAND\",\"reasoning\":\"...\",\
\"new_tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher\"}}],\
\"issues\":\"\",\"refinement_instructions\":\"\"}}\n\
{{\"verdict\":\"REVISE\",\"reasoning\":\"...\",\"new_tasks\":[],\
\"issues\":\"...\",\"refinement_instructions\":\"...\"}}\n\n\
When in doubt, choose SUCCESS."
        );

        match provider.generate_response(&prompt, None).await {
            Ok(ToolResponse::Text(text)) => parse_verdict(&text),
            _ => {
                println!("  ✅ BuiltinGovernor: Quality Controller unavailable — approving mission.");
                ValidationResult::Success
            }
        }
    }

    fn system_prompt_for_role(&self, role: &AgentRole) -> String {
        match role {
            AgentRole::Analyst => {
                "You are an Analyst agent.\n\
- Use 'calculator' for arithmetic.\n\
- Use 'write_file' to save Markdown reports to disk.\n\
- Use 'vision' for image analysis.\n\
- Use 'feedback' to request human input.\n\
- Use 'finalize_mission_state' as your FINAL step to deliver all findings as a \
structured JSON payload. Call it exactly once with a complete structured_data_payload \
object — never write plain text when data is available.\n\
- In design_tokens, set layout_strategy based on data shape:\n\
  * Mostly tables, comparisons, or side-by-side data → \"Bento-Wide\"\n\
  * Narrative article with one hero section + supporting images → \"Magazine-Flow\"\n\
  * Several charts are the main story → \"FocusOnCharts\"\n\
  * Default mixed dashboard → \"Overview\"\n"
                    .to_string()
            }
            AgentRole::Coder => {
                "You are a Coder agent. Write Python 3 code using the 'python_interpreter' tool. \
Use only the standard library and print() for output.\n"
                    .to_string()
            }
            AgentRole::WebSearcher => {
                "You are a WebSearcher agent. Use the 'web_search' tool to find information.\n"
                    .to_string()
            }
            AgentRole::Planner => {
                "You are a Planner agent. Research and plan using available tools.\n".to_string()
            }
        }
    }
}

// ── reset_failed_tasks ────────────────────────────────────────────────────────

/// Reset all `Failed` tasks to `Pending` so the Dispatcher can retry them.
pub fn reset_failed_tasks(tasks: &mut Vec<Task>) {
    for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Failed)) {
        task.status = TaskStatus::Pending;
    }
}
