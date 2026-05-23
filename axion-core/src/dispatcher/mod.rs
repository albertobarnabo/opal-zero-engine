use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::engine::{AiProvider, ToolResponse};
use crate::governor::Governor;
use crate::protocol::{ContextBus, MissionUpdate, Task, TaskStatus, Tool, CTX_OUTPUT_SCHEMA};
use crate::tools::RequestKeys;

// Maximum characters of prior-task context to include in an agent's prompt.
// At ~4 chars/token this is ≈1 500 tokens, leaving ≥2 500 tokens of headroom
// for the agent's own reasoning at the default 4 096 max_tokens ceiling.
const CONTEXT_CHAR_BUDGET: usize = 6_000;

// ── Retry policy constants (overridable via environment variables) ─────────────
const MAX_LLM_RETRIES: u32    = 3;
const RETRY_BASE_DELAY_MS: u64 = 1_000; // 1 s → 2 s → 4 s

fn max_llm_retries() -> u32 {
    std::env::var("AXION_MAX_LLM_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_LLM_RETRIES)
}

fn retry_base_delay_ms() -> u64 {
    std::env::var("AXION_RETRY_BASE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RETRY_BASE_DELAY_MS)
}

/// Build a context string from the bus that fits within `char_budget` characters.
///
/// Prioritises entries whose slug keys are longer (a rough proxy for recency,
/// since later-planned tasks tend to have more descriptive, longer slugs).
/// Individual entries are truncated at 600 characters rather than dropped
/// entirely, so every task always contributes at least a summary.
fn build_context_window(bus: &ContextBus, char_budget: usize) -> String {
    const ENTRY_CAP: usize = 600;

    // Collect all entries and sort longest-key-first (recency heuristic).
    let mut entries: Vec<(&String, &String)> = bus.data.iter().collect();
    entries.sort_by_key(|(k, _)| Reverse(k.len()));

    let mut out   = String::new();
    let mut spent = 0usize;

    for (slug, result) in &entries {
        let body: String = if result.len() > ENTRY_CAP {
            let safe_end = result.floor_char_boundary(ENTRY_CAP);
            format!("{}… [truncated]", &result[..safe_end])
        } else {
            result.to_string()
        };
        let entry = format!("[{}]: {}\n", slug, body);
        if spent + entry.len() > char_budget {
            break;
        }
        out.push_str(&entry);
        spent += entry.len();
    }

    out
}

pub async fn dispatch_tasks(
    tasks: &mut Vec<Task>,
    context: &mut ContextBus,
    provider: &dyn AiProvider,
    governor: &dyn Governor,
    tx: Option<&tokio::sync::mpsc::Sender<MissionUpdate>>,
    keys: &RequestKeys,
) {
    tracing::info!("Dispatcher: checking dependencies and routing to agents");

    // ── 0. Cycle guard ────────────────────────────────────────────────────────
    if let Some(cycle) = detect_cycle(tasks) {
        tracing::warn!(cycle, "Dispatcher: circular dependency detected — affected tasks will never run");
        // Mark every task involved in a cycle as Failed so the Governor can
        // react (retry/repair) rather than hanging forever.
        for task in tasks.iter_mut() {
            if matches!(task.status, TaskStatus::Pending) {
                task.status = TaskStatus::Failed;
                task.result = Some(format!("Circular dependency: {cycle}"));
            }
        }
        return;
    }

    // ── 1. Main DAG loop (sequential execution, dependency-ordered) ───────────
    loop {
        // Collect the slugs of every completed task (owned — avoids borrow conflicts).
        let completed_slugs: HashSet<String> = tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .map(|t| t.slug.clone())
            .collect();

        // Collect the slugs of every failed task (for cascade propagation).
        let failed_slugs: HashSet<String> = tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Failed))
            .map(|t| t.slug.clone())
            .collect();

        // ── Cascade-fail: any Pending task that depends on a failed task ──────
        let pre_cascade_failed = failed_slugs.len();
        let cascade_indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                matches!(t.status, TaskStatus::Pending)
                    && t.depends_on.iter().any(|dep| failed_slugs.contains(dep))
            })
            .map(|(i, _)| i)
            .collect();

        for idx in cascade_indices {
            let failing_dep = tasks[idx]
                .depends_on
                .iter()
                .find(|dep| failed_slugs.contains(dep.as_str()))
                .cloned()
                .unwrap_or_default();
            let slug = tasks[idx].slug.clone();
            let role = tasks[idx].role.as_str().to_string();
            tracing::info!(slug, failing_dep, "cascade-fail: task skipped due to failed dependency");
            tasks[idx].status = TaskStatus::Failed;
            tasks[idx].result = Some(format!("Skipped — dependency '{}' failed", failing_dep));
            if let Some(tx) = tx {
                let _ = tx
                    .send(MissionUpdate::TaskFailed { slug, role })
                    .await;
            }
        }

        // ── Find tasks that are ready: Pending + all deps completed ───────────
        let ready_indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                matches!(t.status, TaskStatus::Pending)
                    && t.depends_on.iter().all(|dep| completed_slugs.contains(dep))
            })
            .map(|(i, _)| i)
            .collect();

        tracing::debug!(count = ready_indices.len(), "tasks ready to execute");

        if ready_indices.is_empty() {
            let still_pending = tasks.iter().any(|t| matches!(t.status, TaskStatus::Pending));
            let cascade_happened = tasks
                .iter()
                .filter(|t| matches!(t.status, TaskStatus::Failed))
                .count() > pre_cascade_failed;

            if still_pending && cascade_happened {
                // New failures just cascaded — loop again to propagate them.
                continue;
            }
            if still_pending {
                // Deadlock: pending tasks exist but none can ever become ready.
                // Fail them all so the mission terminates cleanly.
                tracing::error!("deadlock detected — failing all stuck pending tasks");
                for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Pending)) {
                    task.status = TaskStatus::Failed;
                    task.result = Some("Deadlock: dependency cannot be satisfied (missing or circular)".into());
                }
            }
            break;
        }

        // ── Execute all ready tasks concurrently ──────────────────────────────
        //
        // Strategy: snapshot the context bus (read-only clone) before the
        // batch, fan out all ready tasks as concurrent futures using
        // `futures::future::join_all`, then write results back to the task
        // slots after all futures complete.
        //
        // We use join_all (not tokio::spawn) because the function receives
        // `&dyn AiProvider` / `&dyn Governor` references that are not
        // `'static`, so they cannot be moved into independently spawned
        // tasks.  join_all still achieves true I/O concurrency: while one
        // future is awaiting a network response the runtime polls the others.

        // Mark all ready tasks as Running and emit TaskStarted events before
        // fanning out, so the stream reflects the correct state immediately.
        for &idx in &ready_indices {
            let task = &mut tasks[idx];
            tracing::info!(intent = %task.intent, "executing task");
            task.status = TaskStatus::Running;
            if let Some(tx) = tx {
                let _ = tx
                    .send(MissionUpdate::TaskStarted {
                        slug: task.slug.clone(),
                        role: task.role.as_str().to_string(),
                        intent: task.intent.clone(),
                    })
                    .await;
            }
        }

        // Snapshot context for the concurrent batch (tasks read, not write).
        let context_snapshot = context.clone();

        // Clone each ready task so we can execute them concurrently without
        // holding a mutable borrow on `tasks`.
        let task_snapshots: Vec<(usize, Task)> = ready_indices
            .iter()
            .map(|&idx| (idx, tasks[idx].clone()))
            .collect();

        // Build one future per ready task and await them all concurrently.
        let futs: Vec<_> = task_snapshots
            .into_iter()
            .map(|(idx, mut task_copy)| {
                let ctx = context_snapshot.clone();
                async move {
                    execute_with_role(&mut task_copy, &ctx, provider, governor, keys).await;
                    (idx, task_copy)
                }
            })
            .collect();

        let batch_results = futures::future::join_all(futs).await;

        tracing::info!(count = batch_results.len(), "batch of tasks completed");

        // Write results back, emit outcome events, and update the context bus.
        for (idx, finished_task) in batch_results {
            tasks[idx].status = finished_task.status.clone();
            tasks[idx].result = finished_task.result.clone();

            tracing::info!(slug = %finished_task.slug, status = ?finished_task.status, "task completed");

            if let Some(tx) = tx {
                if matches!(finished_task.status, TaskStatus::Completed) {
                    let _ = tx
                        .send(MissionUpdate::TaskCompleted {
                            slug: finished_task.slug.clone(),
                            role: finished_task.role.as_str().to_string(),
                            result: finished_task.result.clone().unwrap_or_default(),
                        })
                        .await;
                } else if matches!(finished_task.status, TaskStatus::Failed) {
                    let _ = tx
                        .send(MissionUpdate::TaskFailed {
                            slug: finished_task.slug.clone(),
                            role: finished_task.role.as_str().to_string(),
                        })
                        .await;
                }
            }

            // Store agent findings in the context bus (keyed by slug).
            if let Some(ref res) = finished_task.result {
                tracing::debug!(slug = %finished_task.slug, "storing result in context");
                context.data.insert(finished_task.slug.clone(), res.clone());
            }
        }
    }
}

// ── Retry helper ─────────────────────────────────────────────────────────────
//
// Calls `make_call()` up to `max_retries` times with exponential back-off.
// Returns the first `Ok(T)` or the final `Err` after all attempts are
// exhausted.  Permanent failures (`context_length_exceeded`, `invalid_api_key`)
// short-circuit immediately without sleeping.
async fn call_with_retry<F, Fut, T>(
    label: &str,
    max_retries: u32,
    base_delay_ms: u64,
    mut make_call: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    for attempt in 1..=max_retries.max(1) {
        match make_call().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                // Permanent failures — bail without sleeping
                if e.contains("context_length_exceeded") || e.contains("invalid_api_key") {
                    return Err(e);
                }
                if attempt < max_retries {
                    let delay = base_delay_ms * (1u64 << (attempt - 1));
                    tracing::warn!(
                        label,
                        attempt,
                        max_retries,
                        delay_ms = delay,
                        error = %e,
                        "LLM call failed, retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(format!("[retry] {label}: no attempts made (max_retries={max_retries})"))
}

async fn execute_with_role(
    task: &mut Task,
    context: &ContextBus,
    provider: &dyn AiProvider,
    governor: &dyn Governor,
    keys: &RequestKeys,
) {
    tracing::debug!(intent = %task.intent, "agent starting task");

    // ── Per-role model override ───────────────────────────────────────────────
    // Ask the Governor which model this role should use.  If it returns Some(m)
    // and the provider supports runtime model switching, create a role-specific
    // provider instance; otherwise fall back to the original provider.
    let role_model = governor.model_for_role(&task.role);
    let role_provider_box: Option<Box<dyn AiProvider>> = role_model
        .as_deref()
        .and_then(|m| provider.with_text_model(m));
    let effective_provider: &dyn AiProvider = role_provider_box
        .as_deref()
        .unwrap_or(provider);

    if let Some(ref m) = role_model {
        tracing::info!(role = task.role.as_str(), model = m.as_str(), "role model override");
    }

    // ── Schema contract extracted before prompt assembly ─────────────────────
    // When the caller supplied an output schema it MUST lead the Analyst prompt
    // so it overrides the generic component-format examples that follow.
    let schema_contract: Option<String> =
        context.data.get(CTX_OUTPUT_SCHEMA).and_then(|s| {
            serde_json::from_str::<serde_json::Value>(s).ok().and_then(|v| {
                v.as_object().map(|obj| {
                    let keys: String = obj
                        .keys()
                        .map(|k| format!("  - {k}\n"))
                        .collect();

                    match task.role {
                        crate::protocol::AgentRole::Analyst => format!(
                            "🔒 MANDATORY OUTPUT SCHEMA — THIS OVERRIDES ALL EXAMPLES BELOW:\n\
                             The calling application requires data_payload to use EXACTLY these \
                             top-level keys. Any key not in this list will be silently dropped by the UI.\n\
                             {keys}\
                             Do NOT use ChartCard:, MetricCard:, ComparisonTable:, or Timeline: \
                             prefixes. Use the plain key names above verbatim.\n\
                             Omit a key only if you have zero data for it — never invent values.\n\n",
                        ),
                        crate::protocol::AgentRole::Planner => format!(
                            "🔒 REQUIRED OUTPUT SECTIONS:\n\
                             The final Analyst must populate these specific data sections.\n\
                             Create one focused WebSearcher task per section so the Analyst \
                             has the data it needs:\n\
                             {keys}\
                             Do not create generic research tasks — each task must target \
                             one of the sections listed above.\n\n",
                        ),
                        _ => String::new(),
                    }
                })
            })
        })
        .filter(|s| !s.is_empty());

    // ── System prefix — use minimal schema-aware prompt when schema is present ─
    let system_prefix =
        if schema_contract.is_some()
            && matches!(task.role, crate::protocol::AgentRole::Analyst)
        {
            tracing::debug!("Analyst: schema_contract=Some — using schema_analyst_prompt");
            governor.schema_analyst_prompt()
        } else {
            if matches!(task.role, crate::protocol::AgentRole::Analyst) {
                tracing::warn!("Analyst: schema_contract=None — using full system_prompt_for_role");
            }
            governor.system_prompt_for_role(&task.role)
        };

    // ── Build the full prompt: schema contract (if any) + prefix + context + task
    let mut prompt = String::new();
    if let Some(ref contract) = schema_contract {
        prompt.push_str(contract);
    }
    prompt.push_str(&system_prefix);

    if !context.data.is_empty() {
        prompt.push_str("\nPREVIOUS TASK RESULTS:\n");
        prompt.push_str(&build_context_window(context, CONTEXT_CHAR_BUDGET));
    }

    prompt.push_str(&format!("\nTASK: {}", task.intent));

    // Get available tools for this role, then strip any the task has blacklisted
    // (set by the re-planner when a tool has already failed once).
    //
    // Special case: when the task's sole purpose is to call finalize_mission_state,
    // restrict the tool list to ONLY that tool.  Without this guard the agent
    // roams freely — calling `memory`, `calculator`, etc. — and never actually
    // finalises the mission state, causing infinite Governor expansion loops.
    let is_finalize_task = task.intent.to_lowercase().contains("finalize_mission_state");

    let tools: Vec<Tool> = if is_finalize_task {
        let finalize_only: Vec<Tool> = get_tools_for_role(&task.role, keys)
            .into_iter()
            .filter(|t| !task.excluded_tools.contains(&t.name))
            .filter(|t| t.name == "finalize_mission_state")
            .collect();
        // Safety net: if the tool somehow isn't in the role list, use the
        // hard-coded constructor so the agent is never given zero tools.
        if finalize_only.is_empty() {
            vec![Tool::finalize_mission_state()]
        } else {
            finalize_only
        }
    } else {
        get_tools_for_role(&task.role, keys)
            .into_iter()
            .filter(|t| !task.excluded_tools.contains(&t.name))
            .collect()
    };

    let retries    = max_llm_retries();
    let base_delay = retry_base_delay_ms();
    let role_label = task.role.as_str().to_string();

    // Diagnostic: log the exact tool list offered so failures are easy to trace.
    tracing::debug!(
        role = role_label,
        tools = tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
        "tools offered to agent",
    );

    // ── First LLM turn — wrapped in retry loop ────────────────────────────────
    match call_with_retry(
        &role_label,
        retries,
        base_delay,
        || effective_provider.generate_response(&prompt, Some(tools.clone())),
    )
    .await
    {
        Ok(ToolResponse::Text(text)) => {
            tracing::debug!(text, "provider returned text");
            task.result = Some(text);
            task.status = TaskStatus::Completed;
        }
        Ok(ToolResponse::ToolCall { id, name, arguments }) => {
            tracing::info!(tool = name, args = arguments, "executing tool call");
            match crate::tools::execute_tool(&name, &arguments, &task.id.to_string(), keys).await {
                Ok(tool_result) => {
                    tracing::debug!(tool = name, result = tool_result, "tool result");

                    // Terminal tools (e.g. build_dynamic_ui, feedback) produce the final task
                    // result directly — skip the second LLM turn to prevent the model
                    // from paraphrasing the structured JSON output into prose.
                    if crate::tools::is_terminal_tool(&name) {
                        task.result = Some(tool_result);
                        task.status = TaskStatus::Completed;
                    } else {
                        // ── Second LLM turn — also wrapped in retry loop ──────────────────
                        match call_with_retry(
                            &role_label,
                            retries,
                            base_delay,
                            || effective_provider.submit_tool_result(
                                &prompt,
                                Some(tools.clone()),
                                &id,
                                &name,
                                &arguments,
                                &tool_result,
                            ),
                        )
                        .await
                        {
                            Ok(ToolResponse::Text(final_answer)) => {
                                tracing::debug!(final_answer, "final answer from provider");
                                task.result = Some(final_answer);
                                task.status = TaskStatus::Completed;
                            }
                            Ok(ToolResponse::ToolCall { name: n, .. }) => {
                                tracing::debug!(tool = n, "model requested another tool call — using raw result");
                                task.result = Some(tool_result);
                                task.status = TaskStatus::Completed;
                            }
                            Err(err) => {
                                tracing::debug!(error = %err, "submit_tool_result failed");
                                task.status = TaskStatus::Failed;
                            }
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(tool = name, error = %err, "tool execution failed");
                    task.status = TaskStatus::Failed;
                }
            }
        }
        Err(err) => {
            tracing::debug!(error = %err, "provider returned error");
            task.status = TaskStatus::Failed;
        }
    }
}

// ── Cycle detection ──────────────────────────────────────────────────────────

/// Detect a dependency cycle among `tasks` using depth-first search.
///
/// Returns `Some("slug_a → slug_b → slug_a")` describing one cycle,
/// or `None` if the graph is acyclic.
pub(crate) fn detect_cycle(tasks: &[Task]) -> Option<String> {
    // Build slug → depends_on adjacency map.
    let deps: HashMap<String, Vec<String>> = tasks
        .iter()
        .map(|t| (t.slug.clone(), t.depends_on.clone()))
        .collect();

    let mut visited:   HashSet<String> = HashSet::new();
    let mut rec_stack: HashSet<String> = HashSet::new();

    for task in tasks {
        if !visited.contains(&task.slug) {
            if let Some(cycle) = dfs_cycle(&task.slug, &deps, &mut visited, &mut rec_stack) {
                return Some(cycle);
            }
        }
    }
    None
}

fn dfs_cycle(
    node: &str,
    deps: &HashMap<String, Vec<String>>,
    visited:   &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
) -> Option<String> {
    visited.insert(node.to_string());
    rec_stack.insert(node.to_string());

    if let Some(neighbors) = deps.get(node) {
        for neighbor in neighbors {
            if !visited.contains(neighbor) {
                if let Some(cycle) = dfs_cycle(neighbor, deps, visited, rec_stack) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(neighbor) {
                return Some(format!("{} → {}", node, neighbor));
            }
        }
    }

    rec_stack.remove(node);
    None
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentRole, Task, TaskStatus};
    use uuid::Uuid;

    fn make_task(slug: &str, depends_on: Vec<&str>) -> Task {
        Task {
            id: Uuid::new_v4(),
            slug: slug.to_string(),
            intent: slug.to_string(),
            status: TaskStatus::Pending,
            role: AgentRole::Analyst,
            result: None,
            depends_on: depends_on.into_iter().map(|s| s.to_string()).collect(),
            excluded_tools: vec![],
        }
    }

    #[test]
    fn test_detect_cycle_none() {
        // task_b and task_c both depend on task_a — valid diamond-free DAG.
        let tasks = vec![
            make_task("task_a", vec![]),
            make_task("task_b", vec!["task_a"]),
            make_task("task_c", vec!["task_a"]),
        ];
        assert!(detect_cycle(&tasks).is_none());
    }

    #[test]
    fn test_detect_cycle_self_loop() {
        // task_d depends on itself — should be caught.
        let tasks = vec![
            make_task("task_a", vec![]),
            make_task("task_b", vec!["task_a"]),
            make_task("task_c", vec!["task_a"]),
            make_task("task_d", vec!["task_d"]),
        ];
        let result = detect_cycle(&tasks);
        assert!(result.is_some(), "expected Some(cycle) for self-loop");
        assert!(
            result.unwrap().contains("task_d"),
            "cycle description should mention task_d"
        );
    }
}

fn get_tools_for_role(role: &crate::protocol::AgentRole, keys: &RequestKeys) -> Vec<Tool> {
    use crate::protocol::AgentRole;

    // Alpha Vantage tools require an API key to be set — offering them without a
    // key causes the LLM to eagerly call them, the tool fails immediately, and the
    // whole Analyst task is marked Failed.  Only include them when the key exists.
    let has_av_key = keys.alpha_vantage().is_some();

    // Role → canonical tool names (built dynamically so we can gate AV tools).
    //
    // Analyst: web_search + finalize_mission_state + persistence helpers.
    // Alpha Vantage tools are added only when the env key is present.
    let names: Vec<&str> = match role {
        AgentRole::Analyst => {
            let mut v = vec![
                "web_search", "finalize_mission_state", "vision",
                "feedback", "memory_persist", "generate_document", "read_file",
            ];
            if has_av_key {
                v.extend_from_slice(&[
                    "get_company_overview", "get_price_history",
                    "get_income_statement", "get_news_sentiment",
                ]);
            }
            v
        }
        AgentRole::WebSearcher => vec!["web_search"],
        AgentRole::Planner     => vec!["calculator", "web_search", "write_file", "memory", "feedback", "memory_persist"],
        AgentRole::Coder       => vec!["python_interpreter", "write_file"],
    };

    // Use the live registry when available; fall back to hard-coded constructors
    // so tests that skip Registry::init_default() continue to work.
    if let Some(reg) = crate::registry::Registry::get() {
        let tools = reg.tools_for_names(&names);
        if !tools.is_empty() {
            return tools;
        }
    }

    // Hard-coded fallback (used in tests and when manifests directory is absent).
    // WASM-only tools (memory, generate_document, read_file) are omitted here
    // because they have no native implementation.
    match role {
        AgentRole::Analyst     => vec![Tool::web_search(), Tool::finalize_mission_state(), Tool::vision(), Tool::feedback(), Tool::memory_persist()],
        AgentRole::WebSearcher => vec![Tool::web_search()],
        AgentRole::Planner     => vec![Tool::calculator(), Tool::web_search(), Tool::write_file(), Tool::feedback(), Tool::memory_persist()],
        AgentRole::Coder       => vec![Tool::python_interpreter(), Tool::write_file()],
    }
}
