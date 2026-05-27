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
    std::env::var("OPALZERO_MAX_LLM_RETRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_LLM_RETRIES)
}

fn retry_base_delay_ms() -> u64 {
    std::env::var("OPALZERO_RETRY_BASE_DELAY_MS")
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
            let safe_end = (0..=ENTRY_CAP.min(result.len())).rev().find(|&i| result.is_char_boundary(i)).unwrap_or(0);
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
        let role_tools = get_tools_for_role(&task.role, keys);

        // ── Allowlist (per-task skill set) ────────────────────────────────────
        // When the Planner declared an explicit tool list for this task, narrow
        // the role defaults to only those names.  None = use the full role set.
        let after_allowlist: Vec<Tool> = match &task.allowed_tools {
            Some(allowed) if !allowed.is_empty() => {
                let filtered: Vec<Tool> = role_tools
                    .into_iter()
                    .filter(|t| allowed.contains(&t.name))
                    .collect();
                // Safety net: if the allowlist names don't match any registered
                // tool (e.g. a typo in the planner output), fall back to the
                // full role set so the task is never given zero tools.
                if filtered.is_empty() {
                    tracing::warn!(
                        task = %task.slug,
                        allowed = ?allowed,
                        "allowed_tools matched no registered tools — using role defaults"
                    );
                    get_tools_for_role(&task.role, keys)
                } else {
                    tracing::debug!(
                        task = %task.slug,
                        tools = filtered.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
                        "per-task skill set applied"
                    );
                    filtered
                }
            }
            // Empty vec = caller explicitly wants zero tools (pure-reasoning task).
            Some(allowed) if allowed.is_empty() => {
                tracing::debug!(task = %task.slug, "empty allowed_tools — pure-reasoning task, no tools offered");
                vec![]
            }
            // None = no planner constraint, use full role defaults.
            _ => role_tools,
        };

        // ── Blocklist (re-planner exclusions) ─────────────────────────────────
        after_allowlist
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
            // Text-mode fallback: open-source models often return the tool
            // invocation as prose rather than a structured ToolCall.  Attempt
            // to extract and execute a tool call from the text before falling
            // back to treating the response as a plain result.
            if let Some((tool_name, arguments)) = try_extract_tool_call(&text, &tools) {
                tracing::info!(
                    tool = tool_name,
                    "[text-mode] extracted tool call from prose — executing"
                );

                // ── Pre-Write Gate (text-mode path) ───────────────────────────
                if crate::safety::is_write_invocation(&tool_name, &arguments) {
                    match crate::safety::check_write_authorization(
                        &tool_name,
                        &arguments,
                        &task.intent,
                        effective_provider,
                    )
                    .await
                    {
                        crate::safety::WriteDecision::Denied(reason) => {
                            task.status = TaskStatus::Failed;
                            task.result = Some(reason);
                            return;
                        }
                        crate::safety::WriteDecision::Authorized => {}
                    }
                }

                match crate::tools::execute_tool(
                    &tool_name, &arguments, &task.id.to_string(), keys,
                )
                .await
                {
                    Ok(tool_result) => {
                        task.result = Some(tool_result);
                        task.status = TaskStatus::Completed;
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err, tool = tool_name,
                            "[text-mode] tool execution failed — using prose as result"
                        );
                        task.result = Some(text);
                        task.status = TaskStatus::Completed;
                    }
                }
            } else {
                tracing::debug!("[text-mode] no tool call found — using prose as result");
                task.result = Some(text);
                task.status = TaskStatus::Completed;
            }
        }
        Ok(ToolResponse::ToolCall { id, name, arguments }) => {
            tracing::info!(tool = name, args = arguments, "executing tool call");

            // ── Pre-Write Gate ─────────────────────────────────────────────────
            if crate::safety::is_write_invocation(&name, &arguments) {
                match crate::safety::check_write_authorization(
                    &name,
                    &arguments,
                    &task.intent,
                    effective_provider,
                )
                .await
                {
                    crate::safety::WriteDecision::Denied(reason) => {
                        task.status = TaskStatus::Failed;
                        task.result = Some(reason);
                        return;
                    }
                    crate::safety::WriteDecision::Authorized => {}
                }
            }

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

// ── Text-mode tool-call extraction (open-source model fallback) ──────────────
//
// Many Ollama / small open-source models understand tool calling conceptually
// but return the invocation as prose or markdown rather than a structured
// ToolCall API response.  The three functions below scan the raw text for
// recognisable patterns and recover the tool name + arguments so the kernel
// can execute the tool as normal.

/// Walk `s` from the first `{` and extract the complete, balanced JSON object.
/// Handles nested objects and string literals (including escaped quotes).
/// Returns the raw object string (braces included) or `None`.
fn extract_balanced_braces(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let s = &s[start..];
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut end = 0;

    for (i, ch) in s.char_indices() {
        if escape_next { escape_next = false; continue; }
        match ch {
            '\\' if in_string => { escape_next = true; }
            '"'               => { in_string = !in_string; }
            '{' if !in_string => { depth += 1; }
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 { end = i + 1; break; }
            }
            _ => {}
        }
    }

    if end > 0 { Some(s[..end].to_string()) } else { None }
}

/// Heuristically match a parsed JSON value to a known tool by its top-level keys.
fn match_tool_by_keys(val: &serde_json::Value, tools: &[Tool]) -> Option<String> {
    let obj = val.as_object()?;

    // finalize_mission_state — must contain `structured_data_payload`
    if obj.contains_key("structured_data_payload")
        && tools.iter().any(|t| t.name == "finalize_mission_state")
    {
        return Some("finalize_mission_state".to_string());
    }

    // web_search — must contain `query`
    if obj.contains_key("query") && tools.iter().any(|t| t.name == "web_search") {
        return Some("web_search".to_string());
    }

    None
}

/// Try to extract a `(tool_name, arguments_json)` pair from a prose response.
///
/// Strategies (attempted in order):
///  1. Explicit `tool_name({...})` invocation syntax.
///  2. ` ```json ... ``` ` code block whose keys suggest a particular tool.
///  3. Any bare JSON object in the text whose keys suggest a tool (last resort).
///
/// Returns `None` when no confident match is found, leaving the caller free to
/// treat the response as a plain-text result.
fn try_extract_tool_call(text: &str, tools: &[Tool]) -> Option<(String, String)> {
    if tools.is_empty() {
        return None;
    }

    // ── Strategy 1: explicit tool_name({...}) ────────────────────────────────
    for tool in tools {
        let pattern = format!("{}(", tool.name);
        if let Some(pos) = text.find(&pattern) {
            let after_paren = &text[pos + pattern.len()..];
            if let Some(json) = extract_balanced_braces(after_paren) {
                if serde_json::from_str::<serde_json::Value>(&json).is_ok() {
                    return Some((tool.name.clone(), json));
                }
            }
        }
    }

    // ── Strategy 2: ```json code block ───────────────────────────────────────
    let mut remaining = text;
    while let Some(cb_start) = remaining.find("```json") {
        let after_tag = &remaining[cb_start + 7..];
        if let Some(cb_end) = after_tag.find("```") {
            let candidate = after_tag[..cb_end].trim();
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(candidate) {
                if let Some(name) = match_tool_by_keys(&val, tools) {
                    return Some((name, candidate.to_string()));
                }
            }
            remaining = &after_tag[cb_end + 3..];
        } else {
            break;
        }
    }

    // ── Strategy 3: bare JSON object ─────────────────────────────────────────
    let mut search = text;
    while let Some(brace_pos) = search.find('{') {
        if let Some(json) = extract_balanced_braces(&search[brace_pos..]) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(name) = match_tool_by_keys(&val, tools) {
                    return Some((name, json));
                }
            }
        }
        // Advance past this `{` to avoid re-processing.
        search = &search[brace_pos + 1..];
    }

    None
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
            allowed_tools: None,
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

    // ── Additional cycle-detection tests ─────────────────────────────────────

    #[test]
    fn test_detect_cycle_linear_chain_no_cycle() {
        // A → B → C: no cycle.
        let tasks = vec![
            make_task("a", vec![]),
            make_task("b", vec!["a"]),
            make_task("c", vec!["b"]),
        ];
        assert!(detect_cycle(&tasks).is_none(), "linear chain must be acyclic");
    }

    #[test]
    fn test_detect_cycle_two_independent_tasks_no_cycle() {
        // A and B have no relationship at all.
        let tasks = vec![
            make_task("alpha", vec![]),
            make_task("beta", vec![]),
        ];
        assert!(detect_cycle(&tasks).is_none(), "independent tasks must be acyclic");
    }

    #[test]
    fn test_detect_cycle_direct_ab_ba() {
        // A → B and B → A: direct 2-node cycle.
        let tasks = vec![
            make_task("node_a", vec!["node_b"]),
            make_task("node_b", vec!["node_a"]),
        ];
        let result = detect_cycle(&tasks);
        assert!(result.is_some(), "A→B→A must be detected as a cycle");
        let desc = result.unwrap();
        // The description must mention both nodes.
        assert!(
            desc.contains("node_a") || desc.contains("node_b"),
            "cycle description must name at least one involved node; got: {desc}"
        );
    }

    #[test]
    fn test_detect_cycle_longer_loop_abc_a() {
        // A → B → C → A: 3-node cycle.
        let tasks = vec![
            make_task("p", vec!["r"]),
            make_task("q", vec!["p"]),
            make_task("r", vec!["q"]),
        ];
        let result = detect_cycle(&tasks);
        assert!(result.is_some(), "A→B→C→A must be detected as a cycle");
    }

    // ── build_context_window tests ────────────────────────────────────────────

    fn make_bus(entries: &[(&str, &str)]) -> ContextBus {
        let mut bus = ContextBus::default();
        for (k, v) in entries {
            bus.data.insert(k.to_string(), v.to_string());
        }
        bus
    }

    #[test]
    fn context_window_empty_bus_returns_empty_string() {
        let bus = ContextBus::default();
        let out = build_context_window(&bus, 1_000);
        assert_eq!(out, "", "empty bus must produce empty context window");
    }

    #[test]
    fn context_window_single_entry_included() {
        let bus = make_bus(&[("task_one", "hello world")]);
        let out = build_context_window(&bus, 10_000);
        assert!(out.contains("task_one"), "output must contain the slug key");
        assert!(out.contains("hello world"), "output must contain the entry value");
    }

    #[test]
    fn context_window_respects_char_budget() {
        // Build a bus with two entries whose combined formatted size exceeds the
        // tight budget so only the first (longest key, priority) fits.
        let long_val = "x".repeat(50);
        let bus = make_bus(&[
            ("longer_key_here", &long_val),
            ("short", &long_val),
        ]);
        // Budget just enough for one entry but not two.
        // "[longer_key_here]: " + 50 x's + "\n" ≈ 71 chars
        let budget = 80;
        let out = build_context_window(&bus, budget);
        assert!(
            out.len() <= budget,
            "output length {} exceeds budget {budget}",
            out.len()
        );
    }

    #[test]
    fn context_window_long_entry_truncated_not_dropped() {
        // A single entry whose value exceeds ENTRY_CAP (600) must appear truncated.
        let big_val = "z".repeat(800);
        let bus = make_bus(&[("big_task", &big_val)]);
        let out = build_context_window(&bus, 10_000);
        // The entry must be present but not contain the full 800 chars.
        assert!(out.contains("big_task"), "key must appear even when value is truncated");
        assert!(out.contains("… [truncated]"), "truncation marker must be present");
        // The raw 800-char value must NOT appear verbatim.
        assert!(!out.contains(&big_val), "full oversized value must not appear verbatim");
    }

    #[test]
    fn context_window_all_entries_fit_when_budget_large() {
        let bus = make_bus(&[
            ("task_alpha", "result alpha"),
            ("task_beta",  "result beta"),
        ]);
        let out = build_context_window(&bus, 100_000);
        assert!(out.contains("task_alpha"));
        assert!(out.contains("task_beta"));
    }
}

fn get_tools_for_role(role: &crate::protocol::AgentRole, keys: &RequestKeys) -> Vec<Tool> {
    use crate::protocol::AgentRole;
    use crate::tools::send_email::smtp_configured;

    // Alpha Vantage tools require an API key to be set — offering them without a
    // key causes the LLM to eagerly call them, the tool fails immediately, and the
    // whole Analyst task is marked Failed.  Only include them when the key exists.
    let has_av_key   = keys.alpha_vantage().is_some();
    let has_smtp     = smtp_configured();

    // Role → canonical tool names (built dynamically so we can gate optional tools).
    let names: Vec<&str> = match role {
        AgentRole::Analyst => {
            let mut v = vec![
                "web_search", "fetch_page", "http_request", "rss_reader",
                "finalize_mission_state", "vision",
                "feedback", "memory_persist", "generate_document",
                "read_file", "read_csv", "extract_pdf_text",
                "sqlite_query", "diff",
            ];
            if has_av_key {
                v.extend_from_slice(&[
                    "get_company_overview", "get_price_history",
                    "get_income_statement", "get_news_sentiment",
                ]);
            }
            if has_smtp { v.push("send_email"); }
            v
        }
        AgentRole::WebSearcher => vec!["web_search", "fetch_page", "rss_reader"],
        AgentRole::Planner     => vec!["calculator", "web_search", "write_file", "memory", "feedback", "memory_persist"],
        AgentRole::Coder       => vec!["python_interpreter", "write_file", "sqlite_query", "http_request", "diff"],
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
    let mut analyst_tools = vec![
        Tool::web_search(), Tool::fetch_page(), Tool::http_request(), Tool::rss_reader(),
        Tool::finalize_mission_state(), Tool::vision(),
        Tool::feedback(), Tool::memory_persist(),
        Tool::read_csv(), Tool::extract_pdf_text(),
        Tool::sqlite_query(), Tool::diff(),
    ];
    if has_smtp { analyst_tools.push(Tool::send_email()); }

    match role {
        AgentRole::Analyst     => analyst_tools,
        AgentRole::WebSearcher => vec![Tool::web_search(), Tool::fetch_page(), Tool::rss_reader()],
        AgentRole::Planner     => vec![Tool::calculator(), Tool::web_search(), Tool::write_file(), Tool::feedback(), Tool::memory_persist()],
        AgentRole::Coder       => vec![Tool::python_interpreter(), Tool::write_file(), Tool::sqlite_query(), Tool::http_request(), Tool::diff()],
    }
}
