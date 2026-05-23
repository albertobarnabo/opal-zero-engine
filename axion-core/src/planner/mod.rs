use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::engine::{AiProvider, ToolResponse};
use crate::protocol::{AgentRole, ContextBus, Task, TaskStatus};
use crate::tools::RequestKeys;

// Slug format used when building dependency references for the LLM prompt.
// Must match `make_slug()`: first ≤6 significant words, lowercased, joined with "_".
const SLUG_FORMAT_NOTE: &str =
    "A task's slug is derived from its description: take the first ≤6 significant \
     words (skip common stop-words like 'a', 'the', 'for', 'and', 'in'), lowercase \
     them, strip non-alphanumeric characters, and join with underscores. \
     Example: 'Search for flight prices to Tokyo' → 'search_flight_prices_tokyo'.";

#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub original_intent: String,
    pub tasks: Vec<Task>,
    pub context: ContextBus,
    /// Per-request API key overrides.  Not persisted to snapshots.
    #[serde(skip, default)]
    pub keys: RequestKeys,
}

impl Plan {
    pub fn new(intent: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            original_intent: intent.to_string(),
            tasks: Vec::new(),
            context: ContextBus::default(),
            keys: RequestKeys::default(),
        }
    }

    /// Add a task with optional slug-based dependencies.
    /// Pass `vec![]` for tasks that should start immediately.
    /// Returns the task's **slug** — use it to build `depends_on` chains.
    pub fn add_task(&mut self, description: &str, dependencies: Vec<String>, role: AgentRole) -> String {
        self.add_task_excluded(description, dependencies, role, vec![])
    }

    /// Like [`add_task`] but also records tools the agent must not be offered.
    /// Returns the task's **slug**.
    pub fn add_task_excluded(
        &mut self,
        description: &str,
        dependencies: Vec<String>,
        role: AgentRole,
        excluded_tools: Vec<String>,
    ) -> String {
        let id        = Uuid::new_v4();
        let base_slug = crate::util::slugify_unique_fallback(description, 6);

        // ── Slug uniqueness ───────────────────────────────────────────────────
        // If two tasks produce the same base slug (same first 6 significant
        // words), append an incrementing suffix so the cycle detector and
        // dependency resolver can tell them apart.
        let slug = if !self.tasks.iter().any(|t| t.slug == base_slug) {
            base_slug
        } else {
            let mut candidate;
            let mut counter = 2usize;
            loop {
                candidate = format!("{}_{}", base_slug, counter);
                if !self.tasks.iter().any(|t| t.slug == candidate) {
                    break;
                }
                counter += 1;
            }
            candidate
        };

        // Guard: remove any dependency whose slug equals or starts with this
        // task's own slug.  This prevents self-referential depends_on entries
        // that arise when the Governor re-injects a task whose description
        // produces the same slug as a previously completed attempt (e.g. the
        // repeated finalize_mission_state expansion task).  A task that depends
        // on itself can never become ready and causes the cycle detector to fail
        // the entire mission.
        let dependencies: Vec<String> = dependencies
            .into_iter()
            .filter(|dep| !dep.starts_with(slug.as_str()))
            .collect();

        let task = Task {
            id,
            slug: slug.clone(),
            intent: description.to_string(),
            status: TaskStatus::Pending,
            role,
            result: None,
            depends_on: dependencies,
            excluded_tools,
        };
        self.tasks.push(task);
        slug
    }
}

/// Ask the LLM to decompose `intent` into concrete tasks and return a ready [`Plan`].
///
/// Falls back to a single open-ended Analyst task if the LLM returns
/// unparseable output, so the mission always has at least one task to run.
pub async fn build_plan_from_intent(intent: &str, provider: &dyn AiProvider) -> Plan {
    let mut plan = Plan::new(intent);

    let prompt = format!(
        "You are a mission planner. Decompose the user request into 2-5 concrete tasks \
         for specialist agents.\n\n\
         Available roles:\n\
         - WebSearcher: searches the web for factual information\n\
         - Analyst: runs calculations, synthesizes findings, builds dashboards\n\
         - Coder: writes and executes Python code\n\n\
         Rules:\n\
         - Always include a final Analyst task to synthesize the research results.\n\
         - Do NOT add a build_dynamic_ui step — the system injects it automatically.\n\
         - Keep each description specific and actionable (one sentence).\n\n\
         Sequencing — depends_on field:\n\
         - Each task may include an optional \"depends_on\" array listing the slugs of \
           tasks that must complete before this one starts. Omit or use [] if the task \
           can start immediately.\n\
         - If a task needs the output of another task, list that task's slug in depends_on. \
           Example: the final Analyst should depend on all WebSearcher tasks.\n\
         - Never create circular dependencies.\n\
         - {slug_note}\n\n\
         Respond ONLY with valid JSON, no markdown:\n\
         {{\"tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher\",\"depends_on\":[]}}]}}\n\n\
         User intent: {}",
        intent,
        slug_note = SLUG_FORMAT_NOTE,
    );

    if let Ok(ToolResponse::Text(text)) = provider.generate_response(&prompt, None).await {
        #[derive(Deserialize)]
        struct PlanJson { tasks: Vec<TaskJson> }
        #[derive(Deserialize)]
        struct TaskJson {
            description: String,
            role: String,
            #[serde(default)]
            depends_on: Vec<String>,
        }

        let json_str = crate::governor::extract_json(&text).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_str::<PlanJson>(&json_str) {
            if !parsed.tasks.is_empty() {
                for t in &parsed.tasks {
                    let role = match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        "Coder"   => AgentRole::Coder,
                        _         => AgentRole::WebSearcher,
                    };
                    // Add all non-Analyst tasks first with the LLM's depends_on.
                    // Analyst tasks are wired below after we know the real slugs.
                    if !matches!(role, AgentRole::Analyst) {
                        plan.add_task(&t.description, t.depends_on.clone(), role);
                    }
                }

                // ── Fix Bug 1: wire Analyst to actual computed non-Analyst slugs ──
                //
                // The LLM guesses slug strings in depends_on, but those strings
                // rarely match what make_slug() computes from each description.
                // We override every Analyst task's depends_on with the true slugs
                // of every non-Analyst task already in the plan so the Analyst is
                // always schedulable once all WebSearchers have completed.
                let non_analyst_slugs: Vec<String> = plan.tasks
                    .iter()
                    .filter(|t| !matches!(t.role, AgentRole::Analyst))
                    .map(|t| t.slug.clone())
                    .collect();

                for t in &parsed.tasks {
                    if matches!(t.role.as_str(), "Analyst") {
                        plan.add_task(&t.description, non_analyst_slugs.clone(), AgentRole::Analyst);
                    }
                }

                if !plan.tasks.is_empty() {
                    return plan;
                }
            }
        }
    }

    // Fallback: single open-ended Analyst task.
    tracing::warn!("Planner: LLM did not return a valid task list — using single-task fallback");
    plan.add_task(intent, vec![], AgentRole::Analyst);
    plan
}

/// Build a [`Plan`] for a *refinement* pass on a previously-completed mission.
///
/// The prompt is enriched with a plain-text summary of the prior
/// `data_payload` so the planner generates only the incremental tasks
/// needed to address `refinement_intent`, not a fresh mission from scratch.
pub async fn build_refinement_plan(
    original_intent: &str,
    refinement_intent: &str,
    prior_summary: &str,
    provider: &dyn AiProvider,
) -> Plan {
    // The "intent" stored on the plan is the refinement description so that the
    // context bus and snapshot reflect what this refinement actually does.
    let combined_intent = format!("REFINE: {}", refinement_intent);
    let mut plan = Plan::new(&combined_intent);

    let prompt = format!(
        "You are a mission planner. A previous mission has already produced the following findings:\n\
         ---\n\
         {prior_summary}\n\
         ---\n\
         The user now wants to refine/extend those findings with this follow-up request:\n\
         \"{refinement_intent}\"\n\n\
         Original mission intent for context: \"{original_intent}\"\n\n\
         Generate ONLY the incremental tasks required to address the refinement. \
         Do NOT repeat research that is already captured in the existing findings above. \
         Focus exclusively on what is NEW or EXTENDED.\n\n\
         Available roles:\n\
         - WebSearcher: searches the web for factual information\n\
         - Analyst: synthesizes, compares, and finalises findings\n\
         - Coder: writes and executes Python code\n\n\
         Rules:\n\
         - 1-4 tasks maximum (refinements are targeted, not full missions).\n\
         - Always end with an Analyst task that calls finalize_mission_state with \
           BOTH the prior findings AND the new findings merged into one payload.\n\
         - Do NOT add a build_dynamic_ui step.\n\n\
         Sequencing — depends_on field:\n\
         - Each task may include an optional \"depends_on\" array listing the slugs of \
           tasks that must complete before this one starts. Omit or use [] if the task \
           can start immediately.\n\
         - The final Analyst task should depend on all preceding tasks.\n\
         - Never create circular dependencies.\n\
         - {slug_note}\n\n\
         Respond ONLY with valid JSON, no markdown:\n\
         {{\"tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher\",\"depends_on\":[]}}]}}",
        slug_note = SLUG_FORMAT_NOTE,
    );

    if let Ok(ToolResponse::Text(text)) = provider.generate_response(&prompt, None).await {
        #[derive(Deserialize)]
        struct PlanJson { tasks: Vec<TaskJson> }
        #[derive(Deserialize)]
        struct TaskJson {
            description: String,
            role: String,
            #[serde(default)]
            depends_on: Vec<String>,
        }

        let json_str = crate::governor::extract_json(&text).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_str::<PlanJson>(&json_str) {
            if !parsed.tasks.is_empty() {
                for t in parsed.tasks {
                    let role = match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        "Coder"   => AgentRole::Coder,
                        _         => AgentRole::WebSearcher,
                    };
                    plan.add_task(&t.description, t.depends_on, role);
                }
                return plan;
            }
        }
    }

    // Fallback: single Analyst task that merges prior + refinement.
    tracing::warn!("Planner: refinement plan failed — using single-task fallback");
    plan.add_task(
        &format!(
            "Extend the existing mission findings with this refinement: '{}'. \
             Prior data is already in context. Call finalize_mission_state once with \
             both the prior findings and the new findings merged.",
            refinement_intent
        ),
        vec![],
        AgentRole::Analyst,
    );
    plan
}

/// Ask the LLM to generate alternative tasks for failed ones (dynamic re-planning).
///
/// Called by `run_loop` after the second consecutive failure round. Returns an
/// empty Vec when the LLM is unavailable or returns unparseable output — callers
/// should fall back to a plain reset+retry in that case.
pub async fn repair_failed_tasks(
    failed: &[Task],
    original_intent: &str,
    provider: &dyn AiProvider,
) -> Vec<crate::governor::NewTask> {
    if failed.is_empty() {
        return vec![];
    }

    // ── Short-circuit: Analyst failure → direct retry ────────────────────────
    //
    // When any Analyst task failed (whether because of a deadlock, a bad LLM
    // response, or a failed finalize_mission_state call), the correct repair is
    // to re-inject the same Analyst intent — never to re-plan WebSearcher data
    // collection, which regenerates duplicate slugs and causes circular deps.
    //
    // If the task was specifically finalize_mission_state, use a simplified
    // payload prompt as before; otherwise just retry the original intent.
    let analyst_failed = failed.iter().find(|t| matches!(t.role, AgentRole::Analyst));
    if let Some(failed_analyst) = analyst_failed {
        if failed_analyst.intent.contains("finalize_mission_state") {
            tracing::info!("Re-planner: finalize_mission_state failed — injecting targeted retry");
            return vec![crate::governor::NewTask {
                description: "Call finalize_mission_state with a SIMPLIFIED payload. \
Use a flat structured_data_payload with 3-5 scalar keys only (strings or numbers). \
Pull the most important facts from the PREVIOUS TASK RESULTS in your context \
(prices, totals, recommendations) and map each one to a descriptive key. \
Call finalize_mission_state EXACTLY ONCE."
                    .to_string(),
                role: AgentRole::Analyst,
                excluded_tools: vec![],
            }];
        } else {
            tracing::info!("Re-planner: Analyst failed — retrying original Analyst intent directly");
            return vec![crate::governor::NewTask {
                description: failed_analyst.intent.clone(),
                role: AgentRole::Analyst,
                excluded_tools: vec![],
            }];
        }
    }

    let failed_summary: String = failed
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. [{}] \"{}\"", i + 1, t.role.as_str(), t.intent))
        .collect::<Vec<_>>()
        .join("\n");

    // Detect whether the failing tasks included Python execution so we can
    // blacklist `python_interpreter` and steer the LLM toward alternatives.
    let python_failed = failed.iter().any(|t| matches!(t.role, AgentRole::Coder));
    let python_note = if python_failed {
        "\nIMPORTANT: Python execution failed. Do NOT suggest Coder/Python tasks. \
         Use Analyst (calculator tool) or WebSearcher to find the answer instead."
    } else {
        ""
    };

    let prompt = format!(
        "You are a self-healing AI agent. The following tasks failed during a mission and \
         need alternative approaches.\n\n\
         Original mission: {original_intent}\n\n\
         Failed tasks:\n{failed_summary}\n\n\
         Generate 1-3 replacement tasks using a DIFFERENT approach \
         (e.g. if a web search failed, try a different query or reason from context; \
         if a tool errored, break the work into smaller steps or use a simpler method).\
         {python_note}\n\n\
         Respond ONLY with valid JSON, no markdown:\n\
         {{\"tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher|Analyst\"}}]}}"
    );

    if let Ok(ToolResponse::Text(text)) = provider.generate_response(&prompt, None).await {
        #[derive(Deserialize)]
        struct RepairJson {
            tasks: Vec<TaskJson>,
        }
        #[derive(Deserialize)]
        struct TaskJson {
            description: String,
            role: String,
        }

        // When Python failed, blacklist python_interpreter so the repair agent
        // cannot fall back to the same tool even if the LLM tries.
        let excluded: Vec<String> = if python_failed {
            vec!["python_interpreter".to_string()]
        } else {
            vec![]
        };

        let json_str = crate::governor::extract_json(&text).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_str::<RepairJson>(&json_str) {
            let repaired: Vec<crate::governor::NewTask> = parsed
                .tasks
                .into_iter()
                .map(|t| {
                    let role = match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        // Explicitly block the LLM from re-selecting Coder when
                        // Python has already failed — force Analyst instead.
                        "Coder" if python_failed => AgentRole::Analyst,
                        "Coder"   => AgentRole::Coder,
                        _         => AgentRole::WebSearcher,
                    };
                    crate::governor::NewTask {
                        description: t.description,
                        role,
                        excluded_tools: excluded.clone(),
                    }
                })
                .collect();
            if !repaired.is_empty() {
                return repaired;
            }
        }
    }

    vec![]
}

