use crate::protocol::{Task, TaskStatus, ContextBus, Tool};
use crate::engine::{AiProvider, ToolResponse};

pub async fn dispatch_tasks(tasks: &mut Vec<Task>, context: &mut ContextBus, provider: &dyn AiProvider) {
    println!("⚙️  Dispatcher: Checking dependencies and routing to agents...");

    loop {
        let mut spawned = false;
        
        // 1. Identify which tasks are ready (Pending + dependencies met)
        let completed_ids: Vec<uuid::Uuid> = tasks.iter()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .map(|t| t.id)
            .collect();

        // 2. We need to collect IDs of tasks to run to avoid borrow checker issues in the loop
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
            
            // 3. Use the role-based execution logic here
            execute_with_role(task, context, provider).await;
            println!("  ✅ Task completed with status: {:?}", task.status);
            
            // 4. Update the Context Bus with the agent's findings
            if let Some(ref res) = task.result {
                let key = task.intent.replace(" ", "_").to_lowercase();
                println!("  💾 Storing result in context as key: {}", key);
                context.data.insert(key, res.clone());
            }
            
            spawned = true;
        }

        if !spawned { break; }
    }
}

pub async fn execute_with_role(task: &mut Task, context: &ContextBus, provider: &dyn AiProvider) {
    println!("    DEBUG: Agent starting task: {}", task.intent);

    let mut prompt = String::from(
        "You are an autonomous agent in the Axion Core swarm. The user's request is final. \
Do not ask questions. If data like prices or quantities is present in the context, use it \
immediately. Use the 'calculator' tool for any and all math. \
You can persist data to disk. If a task asks to save or write a report, use the 'write_file' tool.\n"
    );

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
        AgentRole::Analyst => vec![Tool::calculator(), Tool::write_file()],
        AgentRole::WebSearcher => vec![Tool::web_search()],
        AgentRole::Planner => vec![Tool::calculator(), Tool::web_search(), Tool::write_file()],
    }
}

