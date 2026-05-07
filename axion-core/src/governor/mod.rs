use crate::engine::{AiProvider, ToolResponse};
use crate::protocol::{AgentRole, Task, TaskStatus};
use serde::Deserialize;

/// The Governor's verdict after reviewing a completed mission round.
pub enum ValidationResult {
    /// All tasks are complete and quality-approved.
    Success,
    /// One or more tasks failed — reset and retry.
    Retry,
    /// Research reveals new requirements; expand the mission with these tasks.
    Expand(Vec<NewTask>),
}

/// A new task the Governor wants to append to the mission.
pub struct NewTask {
    pub description: String,
    pub role: AgentRole,
}

/// Review mission state and consult the AI Quality Controller when all tasks complete.
pub async fn validate_mission(tasks: &[Task], provider: &dyn AiProvider) -> ValidationResult {
    let failed_count = tasks.iter().filter(|t| matches!(t.status, TaskStatus::Failed)).count();
    let completed_count = tasks.iter().filter(|t| matches!(t.status, TaskStatus::Completed)).count();
    let total = tasks.len();

    // Failures take priority — no point consulting AI yet.
    if failed_count > 0 {
        println!("  ⚠️  {} task(s) failed. Marking for retry.", failed_count);
        return ValidationResult::Retry;
    }

    // Guard against stuck tasks (cyclic deps or dispatcher bugs).
    if completed_count < total {
        println!("  ⏳ {}/{} tasks complete — waiting for remaining.", completed_count, total);
        return ValidationResult::Retry;
    }

    println!("\n⚖️  Governor: All {} tasks completed. Consulting Quality Controller...", total);

    // Build a concise mission summary for the AI to review.
    let mut summary = String::from("COMPLETED TASKS:\n");
    for (i, task) in tasks.iter().enumerate() {
        let result_preview = task.result.as_deref().unwrap_or("(no result)");
        let preview = &result_preview[..result_preview.len().min(400)];
        summary.push_str(&format!("\n{}. {}\n   Result: {}…\n", i + 1, task.intent, preview));
    }

    let prompt = format!(
        "You are the Quality Controller for an autonomous AI agent swarm. \
Review the completed mission tasks below and determine if the mission is truly complete, \
or if the results reveal a new requirement that needs investigation — for example: \
a visa is needed, a price is suspiciously low, an insurance requirement was missed, \
or important context is absent from the research.\n\n\
{summary}\n\n\
Respond ONLY with valid JSON in exactly one of these two formats (no markdown, no explanation):\n\
{{\"verdict\":\"SUCCESS\",\"reasoning\":\"brief sentence\",\"new_tasks\":[]}}\n\
{{\"verdict\":\"EXPAND\",\"reasoning\":\"brief sentence\",\"new_tasks\":[\
{{\"description\":\"specific task description\",\"role\":\"WebSearcher\"}}]}}\n\n\
Rules:\n\
- Limit new_tasks to 2 maximum.\n\
- Only EXPAND if something genuinely critical is missing for the user's intent.\n\
- When in doubt, choose SUCCESS."
    );

    match provider.generate_response(&prompt, None).await {
        Ok(ToolResponse::Text(text)) => parse_verdict(&text),
        _ => {
            println!("  ✅ Governor: Quality Controller unavailable — approving mission.");
            ValidationResult::Success
        }
    }
}

// ── JSON parsing helpers ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct VerdictJson {
    verdict: String,
    #[serde(default)]
    new_tasks: Vec<NewTaskJson>,
}

#[derive(Deserialize)]
struct NewTaskJson {
    description: String,
    #[serde(default)]
    role: String,
}

fn parse_verdict(response: &str) -> ValidationResult {
    let json = extract_json(response);
    match serde_json::from_str::<VerdictJson>(&json) {
        Ok(v) if v.verdict == "EXPAND" && !v.new_tasks.is_empty() => {
            let tasks: Vec<NewTask> = v
                .new_tasks
                .into_iter()
                .map(|t| NewTask {
                    description: t.description,
                    role: match t.role.as_str() {
                        "Analyst" => AgentRole::Analyst,
                        "Planner" => AgentRole::Planner,
                        _ => AgentRole::WebSearcher,
                    },
                })
                .collect();
            println!("  🔍 Governor: Expanding mission with {} new task(s).", tasks.len());
            for t in &tasks {
                println!("  ➕ New task [{}]: {}", t.role.as_str(), t.description);
            }
            ValidationResult::Expand(tasks)
        }
        Ok(_) => {
            println!("  ✅ Governor: Mission quality approved (SUCCESS).");
            ValidationResult::Success
        }
        Err(e) => {
            println!("  ⚠️  Governor: Could not parse verdict ('{}') — defaulting to SUCCESS.", e);
            ValidationResult::Success
        }
    }
}

/// Strip markdown code fences and find the first JSON object in the response.
fn extract_json(text: &str) -> String {
    let stripped = text.trim();
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}')) {
        return stripped[start..=end].to_string();
    }
    stripped.to_string()
}

/// Reset all `Failed` tasks to `Pending` so the Dispatcher can retry them.
pub fn reset_failed_tasks(tasks: &mut Vec<Task>) {
    for task in tasks.iter_mut().filter(|t| matches!(t.status, TaskStatus::Failed)) {
        task.status = TaskStatus::Pending;
    }
}
