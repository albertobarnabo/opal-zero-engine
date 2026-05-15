use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::engine::{AiProvider, ToolResponse};
use crate::governor::Governor;
use crate::protocol::{ContextBus, MissionUpdate, Task, TaskStatus, Tool};

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
            format!("{}… [truncated]", &result[..ENTRY_CAP])
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
) {
    println!("⚙️  Dispatcher: Checking dependencies and routing to agents...");

    // ── 0. Cycle guard ────────────────────────────────────────────────────────
    if let Some(cycle) = detect_cycle(tasks) {
        eprintln!("  ⚠️  Dispatcher: circular dependency detected — {cycle}. Affected tasks will never run.");
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
            println!("  ⛔ Cascade-fail: '{}' skipped (dependency '{}' failed)", slug, failing_dep);
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

        println!("  📋 Found {} tasks ready to execute", ready_indices.len());

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
                eprintln!("[dispatcher] deadlock detected — failing all stuck pending tasks");
                for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Pending)) {
                    task.status = TaskStatus::Failed;
                    task.result = Some("Deadlock: dependency cannot be satisfied (missing or circular)".into());
                }
            }
            break;
        }

        // ── Execute each ready task sequentially ──────────────────────────────
        for idx in ready_indices {
            let task = &mut tasks[idx];
            println!("  🔄 Executing task: {}", task.intent);
            task.status = TaskStatus::Running;

            // Notify the stream that this agent is now working.
            if let Some(tx) = tx {
                let _ = tx
                    .send(MissionUpdate::TaskStarted {
                        slug: task.slug.clone(),
                        role: task.role.as_str().to_string(),
                        intent: task.intent.clone(),
                    })
                    .await;
            }

            execute_with_role(task, context, provider, governor).await;
            println!("  ✅ Task completed with status: {:?}", task.status);

            // Emit the outcome event.
            if let Some(tx) = tx {
                if matches!(task.status, TaskStatus::Completed) {
                    let _ = tx
                        .send(MissionUpdate::TaskCompleted {
                            slug: task.slug.clone(),
                            role: task.role.as_str().to_string(),
                            result: task.result.clone().unwrap_or_default(),
                        })
                        .await;
                } else if matches!(task.status, TaskStatus::Failed) {
                    let _ = tx
                        .send(MissionUpdate::TaskFailed {
                            slug: task.slug.clone(),
                            role: task.role.as_str().to_string(),
                        })
                        .await;
                }
            }

            // Store agent findings in the context bus (keyed by slug).
            if let Some(ref res) = task.result {
                println!("  💾 Storing result in context as key: {}", task.slug);
                context.data.insert(task.slug.clone(), res.clone());
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
                    eprintln!(
                        "[retry] {label} attempt {attempt}/{max_retries} failed: {e}. \
                         Retrying in {delay}ms"
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
) {
    println!("    DEBUG: Agent starting task: {}", task.intent);

    // ── System prefix comes from the Governor (pluggable, role-specific) ──────
    let system_prefix = governor.system_prompt_for_role(&task.role);

    // ── Build the full prompt: prefix + context + task ────────────────────────
    let mut prompt = system_prefix;

    if !context.data.is_empty() {
        prompt.push_str("\nPREVIOUS TASK RESULTS:\n");
        prompt.push_str(&build_context_window(context, CONTEXT_CHAR_BUDGET));
    }

    prompt.push_str(&format!("\nTASK: {}", task.intent));
    println!("    DEBUG: Generated prompt: {}", prompt);

    // Get available tools for this role, then strip any the task has blacklisted
    // (set by the re-planner when a tool has already failed once).
    //
    // Special case: when the task's sole purpose is to call finalize_mission_state,
    // restrict the tool list to ONLY that tool.  Without this guard the agent
    // roams freely — calling `memory`, `calculator`, etc. — and never actually
    // finalises the mission state, causing infinite Governor expansion loops.
    let is_finalize_task = task.intent.to_lowercase().contains("finalize_mission_state");

    let tools: Vec<Tool> = if is_finalize_task {
        let finalize_only: Vec<Tool> = get_tools_for_role(&task.role)
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
        get_tools_for_role(&task.role)
            .into_iter()
            .filter(|t| !task.excluded_tools.contains(&t.name))
            .collect()
    };

    let retries    = max_llm_retries();
    let base_delay = retry_base_delay_ms();
    let role_label = task.role.as_str().to_string();

    // ── First LLM turn — wrapped in retry loop ────────────────────────────────
    match call_with_retry(
        &role_label,
        retries,
        base_delay,
        || provider.generate_response(&prompt, Some(tools.clone())),
    )
    .await
    {
        Ok(ToolResponse::Text(text)) => {
            println!("    DEBUG: Provider returned text: {}", text);
            task.result = Some(text);
            task.status = TaskStatus::Completed;
        }
        Ok(ToolResponse::ToolCall { id, name, arguments }) => {
            println!("    🛠️ Executing {} with args: {}", name, arguments);
            match crate::tools::execute_tool(&name, &arguments, &task.id.to_string()).await {
                Ok(tool_result) => {
                    println!("    🔧 Tool Result: {}", tool_result);

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
                            || provider.submit_tool_result(
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
                                println!("    DEBUG: Final answer: {}", final_answer);
                                task.result = Some(final_answer);
                                task.status = TaskStatus::Completed;
                            }
                            Ok(ToolResponse::ToolCall { name: n, .. }) => {
                                println!(
                                    "    DEBUG: Model requested another tool call ({}) — using raw result",
                                    n
                                );
                                task.result = Some(tool_result);
                                task.status = TaskStatus::Completed;
                            }
                            Err(err) => {
                                println!("    DEBUG: submit_tool_result failed: {}", err);
                                task.status = TaskStatus::Failed;
                            }
                        }
                    }
                }
                Err(err) => {
                    println!("    DEBUG: Tool {} failed: {}", name, err);
                    task.status = TaskStatus::Failed;
                }
            }
        }
        Err(err) => {
            println!("    DEBUG: Provider returned Err: {}", err);
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

fn get_tools_for_role(role: &crate::protocol::AgentRole) -> Vec<Tool> {
    use crate::protocol::AgentRole;

    // Role → canonical tool names.
    //
    // Analyst: web_search + finalize_mission_state + persistence helpers only.
    // calculator and memory are intentionally excluded: the Analyst's job is to
    // synthesise and finalise findings, not to do arithmetic or replay memory
    // (those were causing the agent to call the wrong tool for finalize tasks).
    let names: &[&str] = match role {
        AgentRole::Analyst     => &["web_search", "finalize_mission_state", "vision", "feedback", "memory_persist", "generate_document", "read_file"],
        AgentRole::WebSearcher => &["web_search"],
        AgentRole::Planner     => &["calculator", "web_search", "write_file", "memory", "feedback", "memory_persist"],
        AgentRole::Coder       => &["python_interpreter", "write_file"],
    };

    // Use the live registry when available; fall back to hard-coded constructors
    // so tests that skip Registry::init_default() continue to work.
    if let Some(reg) = crate::registry::Registry::get() {
        let tools = reg.tools_for_names(names);
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
