use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::engine::{AiProvider, ToolResponse};
use crate::protocol::{AgentRole, ContextBus, Task, TaskStatus};

#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: Uuid,
    pub original_intent: String,
    pub tasks: Vec<Task>,
    pub context: ContextBus,
}

impl Plan {
    pub fn new(intent: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            original_intent: intent.to_string(),
            tasks: Vec::new(),
            context: ContextBus::default(),
        }
    }

    pub fn add_task(&mut self, description: &str, dependencies: Vec<Uuid>, role: AgentRole) -> Uuid {
        self.add_task_excluded(description, dependencies, role, vec![])
    }

    /// Like [`add_task`] but also records tools the agent must not be offered.
    pub fn add_task_excluded(
        &mut self,
        description: &str,
        dependencies: Vec<Uuid>,
        role: AgentRole,
        excluded_tools: Vec<String>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let task = Task {
            id,
            slug: make_slug(description),
            intent: description.to_string(),
            status: TaskStatus::Pending,
            role,
            result: None,
            depends_on: dependencies,
            excluded_tools,
        };
        self.tasks.push(task);
        id
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
         Respond ONLY with valid JSON, no markdown:\n\
         {{\"tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher\"}}]}}\n\n\
         User intent: {}",
        intent
    );

    if let Ok(ToolResponse::Text(text)) = provider.generate_response(&prompt, None).await {
        #[derive(Deserialize)]
        struct PlanJson { tasks: Vec<TaskJson> }
        #[derive(Deserialize)]
        struct TaskJson { description: String, role: String }

        let json_str = crate::governor::extract_json(&text).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_str::<PlanJson>(&json_str) {
            if !parsed.tasks.is_empty() {
                let mut prev_ids: Vec<Uuid> = vec![];
                for t in parsed.tasks {
                    let role = match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        "Coder"   => AgentRole::Coder,
                        _         => AgentRole::WebSearcher,
                    };
                    let id = plan.add_task(&t.description, prev_ids.clone(), role);
                    prev_ids.push(id);
                }
                return plan;
            }
        }
    }

    // Fallback: single open-ended Analyst task.
    println!("  ⚠️  Planner: LLM did not return a valid task list — using single-task fallback.");
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
         Respond ONLY with valid JSON, no markdown:\n\
         {{\"tasks\":[{{\"description\":\"...\",\"role\":\"WebSearcher\"}}]}}"
    );

    if let Ok(ToolResponse::Text(text)) = provider.generate_response(&prompt, None).await {
        #[derive(Deserialize)]
        struct PlanJson { tasks: Vec<TaskJson> }
        #[derive(Deserialize)]
        struct TaskJson { description: String, role: String }

        let json_str = crate::governor::extract_json(&text).unwrap_or_default();
        if let Ok(parsed) = serde_json::from_str::<PlanJson>(&json_str) {
            if !parsed.tasks.is_empty() {
                let mut prev_ids: Vec<uuid::Uuid> = vec![];
                for t in parsed.tasks {
                    let role = match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        "Coder"   => AgentRole::Coder,
                        _         => AgentRole::WebSearcher,
                    };
                    let id = plan.add_task(&t.description, prev_ids.clone(), role);
                    prev_ids.push(id);
                }
                return plan;
            }
        }
    }

    // Fallback: single Analyst task that merges prior + refinement.
    println!("  ⚠️  Planner: refinement plan failed — using single-task fallback.");
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

    // Short-circuit: if the failing task was finalize_mission_state, inject a
    // targeted retry with a simplified payload rather than asking the LLM for
    // generic alternatives (which tend to change the topic entirely).
    let finalize_failed = failed
        .iter()
        .any(|t| matches!(t.role, AgentRole::Analyst) && t.intent.contains("finalize_mission_state"));
    if finalize_failed {
        println!("  🧠 Re-planner: finalize_mission_state failed — injecting targeted retry.");
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

/// Derive a short, stable, location-aware key from a task intent.
///
/// Takes up to 8 significant words (skipping common stop-words), lowercases
/// them, strips non-alphanumeric characters, and joins with underscores.
/// Ensures uniqueness within a plan because callers include the location in
/// their intent strings (e.g. "Find hotels in Seoul").
fn make_slug(intent: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "in", "on", "at", "to", "for", "of", "and", "or",
        "is", "are", "be", "this", "that", "it", "with", "from", "by",
        "return", "exactly", "using", "use", "report", "fact",
    ];

    let words: Vec<String> = intent
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.len() > 1)
        .take(6)
        .collect();

    if words.is_empty() {
        return uuid::Uuid::new_v4().to_string()[..8].to_string();
    }
    words.join("_")
}