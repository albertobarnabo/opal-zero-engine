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
    pub use crate::governor::{NewTask, ValidationResult};
    pub use crate::planner::Plan;
    pub use crate::protocol::{AgentRole, ContextBus, Task, TaskStatus};
    pub use crate::run_mission;
}

const MAX_EXPANSIONS: u8 = 2;

/// Execute a pre-built [`Plan`] against an AI provider, retrying failed tasks
/// up to `max_attempts` times and allowing the Governor to expand the mission
/// up to `MAX_EXPANSIONS` times when new requirements are discovered.
///
/// Returns `Ok(())` when every task completes successfully, or an `Err` string
/// describing how many tasks could not be completed.
pub async fn run_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    max_attempts: u8,
) -> Result<(), String> {
    // Wipe any context from a previous run on this plan object. This is a
    // defensive clear — the server creates a fresh Plan per request, but
    // explicit cleanup prevents leaks if the caller ever reuses a Plan.
    plan.context.clear();

    let mut retry_attempts: u8 = 0;
    let mut expansion_rounds: u8 = 0;

    loop {
        dispatcher::dispatch_tasks(&mut plan.tasks, &mut plan.context, provider).await;

        match governor::validate_mission(&plan.tasks, provider).await {
            governor::ValidationResult::Success => {
                return Ok(());
            }

            governor::ValidationResult::Retry => {
                governor::reset_failed_tasks(&mut plan.tasks);
                retry_attempts += 1;
                if retry_attempts >= max_attempts {
                    break;
                }
                println!("🚨 Failure detected. Attempting self-healing ({}/{})...", retry_attempts, max_attempts);
            }

            governor::ValidationResult::Expand(new_tasks) => {
                if expansion_rounds >= MAX_EXPANSIONS {
                    println!("  ⚠️  Governor: Max expansions ({}) reached — treating as SUCCESS.", MAX_EXPANSIONS);
                    return Ok(());
                }

                // All currently completed tasks become dependencies for the new tasks,
                // so they run only after the existing mission has fully settled.
                let completed_ids: Vec<uuid::Uuid> = plan.tasks.iter()
                    .map(|t| t.id)
                    .collect();

                println!("\n🔭 Governor: Expanding mission with {} new task(s) (round {}/{}).",
                    new_tasks.len(), expansion_rounds + 1, MAX_EXPANSIONS);

                for nt in new_tasks {
                    plan.add_task(&nt.description, completed_ids.clone(), nt.role);
                }

                expansion_rounds += 1;
            }
        }
    }

    let unfinished = plan.tasks.iter()
        .filter(|t| !matches!(t.status, protocol::TaskStatus::Completed))
        .count();

    Err(format!("{} task(s) could not be completed after {} attempts", unfinished, max_attempts))
}
