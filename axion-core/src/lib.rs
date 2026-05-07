pub mod dispatcher;
pub mod engine;
pub mod governor;
pub mod planner;
pub mod protocol;
pub mod tools;

/// Everything a downstream consumer needs to run a mission in one import.
///
/// ```rust
/// use axion_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::engine::{AiProvider, OpenAIProvider, ToolResponse};
    pub use crate::planner::Plan;
    pub use crate::protocol::{AgentRole, ContextBus, Task, TaskStatus};
    pub use crate::run_mission;
}

/// Execute a pre-built [`Plan`] against an AI provider, retrying failed tasks
/// up to `max_attempts` times.
///
/// Returns `Ok(())` when every task completes successfully, or an `Err` string
/// describing how many tasks could not be completed.
pub async fn run_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    max_attempts: u8,
) -> Result<(), String> {
    let mut attempts = 0;

    while attempts < max_attempts {
        dispatcher::dispatch_tasks(&mut plan.tasks, &mut plan.context, provider).await;

        let report = governor::review_execution(&plan.tasks);

        if report.all_successful {
            return Ok(());
        }

        if !report.failed_tasks.is_empty() {
            governor::reset_failed_tasks(&mut plan.tasks);
        }

        attempts += 1;
        if attempts < max_attempts && !report.failed_tasks.is_empty() {
            println!("🚨 Failure detected. Attempting self-healing ({}/{})...", attempts, max_attempts);
        }
    }

    let unfinished = plan.tasks.iter()
        .filter(|t| !matches!(t.status, protocol::TaskStatus::Completed))
        .count();

    Err(format!("{} task(s) could not be completed after {} attempts", unfinished, max_attempts))
}
