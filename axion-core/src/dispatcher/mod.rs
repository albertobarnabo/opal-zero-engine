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
            execute_with_role(task, provider).await;
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

pub async fn execute_with_role(task: &mut Task, provider: &dyn AiProvider) {
    println!("    DEBUG: Agent starting task: {}", task.intent);
    
    let prompt = format!("You are an AI {}. Task: {}", task.role.as_str(), task.intent);
    println!("    DEBUG: Generated prompt: {}", prompt);
    
    // Get available tools for this role
    let tools = get_tools_for_role(&task.role);
    
    match provider.generate_response(&prompt, Some(tools)).await {
        Ok(ToolResponse::Text(text)) => {
            println!("    DEBUG: Provider returned text: {}", text);
            task.result = Some(text);
            task.status = TaskStatus::Completed;
            println!("    DEBUG: Task status set to Completed");
        }
        Ok(ToolResponse::ToolCall { name, arguments }) => {
            println!("    DEBUG: Provider requested tool call: {} with args: {}", name, arguments);
            match execute_tool(&name, &arguments).await {
                Ok(result) => {
                    println!("    DEBUG: Tool {} returned: {}", name, result);
                    println!("    🔧 Tool Result: {}", result);
                    task.result = Some(result);
                    task.status = TaskStatus::Completed;
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
        AgentRole::Analyst => vec![Tool::calculator()],
        AgentRole::WebSearcher => vec![Tool::web_search()],
        AgentRole::Planner => vec![Tool::calculator(), Tool::web_search()],
    }
}

pub async fn execute_tool(name: &str, arguments: &str) -> Result<String, String> {
    match name {
        "calculator" => execute_calculator(arguments),
        "web_search" => execute_web_search(arguments).await,
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn execute_calculator(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct CalcArgs {
        operation: String,
        #[serde(default)]
        values: Vec<f64>,
    }
    
    let args: CalcArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse calculator arguments: {}", e))?;
    
    if args.values.is_empty() {
        return Err("No values provided for calculation".to_string());
    }
    
    let result = match args.operation.as_str() {
        "add" => args.values.iter().sum::<f64>(),
        "subtract" => args.values.iter().fold(args.values[0], |acc, &x| acc - x),
        "multiply" => args.values.iter().fold(1.0, |acc, &x| acc * x),
        "divide" => {
            let mut result = args.values[0];
            for &val in &args.values[1..] {
                if val == 0.0 {
                    return Err("Division by zero".to_string());
                }
                result /= val;
            }
            result
        }
        _ => return Err(format!("Unknown operation: {}", args.operation)),
    };
    
    Ok(format!("Calculation result: {}", result))
}

pub async fn execute_web_search(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct SearchArgs {
        query: String,
    }
    
    let args: SearchArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse search arguments: {}", e))?;
    
    // Simulated web search - in production, would call a real search API
    Ok(format!("Web search results for '{}': [Simulated results showing top 3 results for the query]", args.query))
}
