use crate::engine::{AiProvider, ToolResponse};
use crate::governor::Governor;
use crate::protocol::{ContextBus, MissionUpdate, Task, TaskStatus, Tool};

pub async fn dispatch_tasks(
    tasks: &mut Vec<Task>,
    context: &mut ContextBus,
    provider: &dyn AiProvider,
    governor: &dyn Governor,
    tx: Option<&tokio::sync::mpsc::Sender<MissionUpdate>>,
) {
    println!("⚙️  Dispatcher: Checking dependencies and routing to agents...");

    loop {
        let mut spawned = false;

        // 1. Identify which tasks are ready (Pending + dependencies met)
        let completed_ids: Vec<uuid::Uuid> = tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .map(|t| t.id)
            .collect();

        // 2. Collect indices of ready tasks to avoid borrow-checker conflicts.
        let ready_indices: Vec<usize> = tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                matches!(t.status, TaskStatus::Pending)
                    && t.depends_on.iter().all(|dep_id| completed_ids.contains(dep_id))
            })
            .map(|(i, _)| i)
            .collect();

        println!("  📋 Found {} tasks ready to execute", ready_indices.len());

        if ready_indices.is_empty() {
            break;
        }

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

            // Update the Context Bus with the agent's findings, keyed by the
            // task's location-aware slug rather than the raw intent string.
            if let Some(ref res) = task.result {
                println!("  💾 Storing result in context as key: {}", task.slug);
                context.data.insert(task.slug.clone(), res.clone());
            }

            spawned = true;
        }

        if !spawned {
            break;
        }
    }
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
        for (key, value) in &context.data {
            prompt.push_str(&format!("{}: {}\n", key, value));
        }
    }

    prompt.push_str(&format!("\nTASK: {}", task.intent));
    println!("    DEBUG: Generated prompt: {}", prompt);

    // Get available tools for this role (cloned so we can reuse for the follow-up turn).
    let tools = get_tools_for_role(&task.role);
    let tools_clone = tools.clone();

    match provider.generate_response(&prompt, Some(tools)).await {
        Ok(ToolResponse::Text(text)) => {
            println!("    DEBUG: Provider returned text: {}", text);
            task.result = Some(text);
            task.status = TaskStatus::Completed;
        }
        Ok(ToolResponse::ToolCall { id, name, arguments }) => {
            println!("    🛠️ Executing {} with args: {}", name, arguments);
            match crate::tools::execute_tool(&name, &arguments).await {
                Ok(tool_result) => {
                    println!("    🔧 Tool Result: {}", tool_result);

                    // Terminal tools (e.g. build_dynamic_ui, feedback) produce the final task
                    // result directly — skip the second LLM turn to prevent the model
                    // from paraphrasing the structured JSON output into prose.
                    if crate::tools::is_terminal_tool(&name) {
                        task.result = Some(tool_result);
                        task.status = TaskStatus::Completed;
                    } else {
                        // Send the result back to the provider to get the final natural-language answer.
                        match provider
                            .submit_tool_result(
                                &prompt,
                                Some(tools_clone),
                                &id,
                                &name,
                                &arguments,
                                &tool_result,
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

fn get_tools_for_role(role: &crate::protocol::AgentRole) -> Vec<Tool> {
    use crate::protocol::AgentRole;

    // Role → canonical tool names.
    // `memory` is included for roles that may need to reference past missions.
    let names: &[&str] = match role {
        AgentRole::Analyst     => &["calculator", "write_file", "build_dynamic_ui", "memory", "vision", "feedback"],
        AgentRole::WebSearcher => &["web_search"],
        AgentRole::Planner     => &["calculator", "web_search", "write_file", "memory", "feedback"],
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
    // `memory` is excluded here because it has no native implementation — it
    // always requires the Wasm binary to be present.
    match role {
        AgentRole::Analyst     => vec![Tool::calculator(), Tool::write_file(), Tool::build_dynamic_ui(), Tool::vision(), Tool::feedback()],
        AgentRole::WebSearcher => vec![Tool::web_search()],
        AgentRole::Planner     => vec![Tool::calculator(), Tool::web_search(), Tool::write_file(), Tool::feedback()],
        AgentRole::Coder       => vec![Tool::python_interpreter(), Tool::write_file()],
    }
}
