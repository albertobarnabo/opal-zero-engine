use crate::protocol::{Task, TaskStatus, ContextBus, MissionUpdate, Tool};
use crate::engine::{AiProvider, ToolResponse};

pub async fn dispatch_tasks(
    tasks: &mut Vec<Task>,
    context: &mut ContextBus,
    provider: &dyn AiProvider,
    tx: Option<&tokio::sync::mpsc::Sender<MissionUpdate>>,
) {
    println!("⚙️  Dispatcher: Checking dependencies and routing to agents...");

    loop {
        let mut spawned = false;

        // 1. Identify which tasks are ready (Pending + dependencies met)
        let completed_ids: Vec<uuid::Uuid> = tasks.iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .map(|t| t.id)
            .collect();

        // 2. Collect indices of ready tasks to avoid borrow-checker conflicts.
        let ready_indices: Vec<usize> = tasks.iter().enumerate()
            .filter(|(_, t)| {
                matches!(t.status, TaskStatus::Pending) &&
                t.depends_on.iter().all(|dep_id| completed_ids.contains(dep_id))
            })
            .map(|(i, _)| i)
            .collect();

        println!("  📋 Found {} tasks ready to execute", ready_indices.len());

        if ready_indices.is_empty() { break; }

        for idx in ready_indices {
            let task = &mut tasks[idx];
            println!("  🔄 Executing task: {}", task.intent);
            task.status = TaskStatus::Running;

            // Notify the stream that this agent is now working.
            if let Some(tx) = tx {
                let _ = tx.send(MissionUpdate::TaskStarted {
                    slug: task.slug.clone(),
                    role: task.role.as_str().to_string(),
                    intent: task.intent.clone(),
                }).await;
            }

            execute_with_role(task, context, provider).await;
            println!("  ✅ Task completed with status: {:?}", task.status);

            // Emit the outcome event.
            if let Some(tx) = tx {
                if matches!(task.status, TaskStatus::Completed) {
                    let _ = tx.send(MissionUpdate::TaskCompleted {
                        slug: task.slug.clone(),
                        role: task.role.as_str().to_string(),
                        result: task.result.clone().unwrap_or_default(),
                    }).await;
                } else if matches!(task.status, TaskStatus::Failed) {
                    let _ = tx.send(MissionUpdate::TaskFailed {
                        slug: task.slug.clone(),
                        role: task.role.as_str().to_string(),
                    }).await;
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

        if !spawned { break; }
    }
}

async fn execute_with_role(task: &mut Task, context: &ContextBus, provider: &dyn AiProvider) {
    use crate::protocol::AgentRole;
    println!("    DEBUG: Agent starting task: {}", task.intent);

    let system_prefix: &str = match task.role {
        AgentRole::Analyst => {
            "You are a Senior Travel Architect with 20 years of experience. \
Your analysis is authoritative — do not hedge or ask questions. \
When you receive cost data from context, analyze the value, suggest alternatives, \
and format your response using professional Markdown: **bold** key figures, \
use tables for cost comparisons, and ## headings to structure sections. \
Use the 'calculator' tool for all arithmetic. \
Use the 'write_file' tool when asked to save a report.\n"
        }
        AgentRole::Coder => {
            "You are a Python programmer in the Axion Core swarm. \
Your job is to write and execute Python 3 code using the 'python_interpreter' tool. \
Rules: use only the standard library, always use print() to emit results, \
never use file I/O or shell commands, keep scripts concise and self-contained. \
Call the 'python_interpreter' tool exactly once with your complete script. \
Use 'write_file' only if explicitly asked to persist the output.\n"
        }
        _ => {
            "You are an autonomous agent in the Axion Core swarm. The user's request is final. \
Do not ask questions. If data like prices or quantities is present in the context, use it \
immediately. Use the 'calculator' tool for any and all math. \
You can persist data to disk. If a task asks to save or write a report, use the 'write_file' tool.\n"
        }
    };

    let mut prompt = String::from(system_prefix);

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
                    // Send the result back to OpenAI to get the final natural-language answer.
                    match provider.submit_tool_result(&prompt, Some(tools_clone), &id, &name, &arguments, &tool_result).await {
                        Ok(ToolResponse::Text(final_answer)) => {
                            println!("    DEBUG: Final answer: {}", final_answer);
                            task.result = Some(final_answer);
                            task.status = TaskStatus::Completed;
                        }
                        Ok(ToolResponse::ToolCall { name: n, .. }) => {
                            println!("    DEBUG: Model requested another tool call ({}) — using raw result", n);
                            task.result = Some(tool_result);
                            task.status = TaskStatus::Completed;
                        }
                        Err(err) => {
                            println!("    DEBUG: submit_tool_result failed: {}", err);
                            task.status = TaskStatus::Failed;
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
    match role {
        AgentRole::Analyst  => vec![Tool::calculator(), Tool::write_file()],
        AgentRole::WebSearcher => vec![Tool::web_search()],
        AgentRole::Planner  => vec![Tool::calculator(), Tool::web_search(), Tool::write_file()],
        AgentRole::Coder    => vec![Tool::python_interpreter(), Tool::write_file()],
    }
}

