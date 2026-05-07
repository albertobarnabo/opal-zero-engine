pub mod dispatcher;
pub mod engine;
pub mod governor;
pub mod persistence;
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
    pub use crate::persistence::MissionSnapshot;
    pub use crate::planner::Plan;
    pub use crate::protocol::{AgentRole, ContextBus, MissionUpdate, Task, TaskStatus};
    pub use crate::run_mission;
}

const MAX_EXPANSIONS: u8 = 2;

/// Execute a pre-built [`Plan`] against an AI provider.
///
/// - Retries failed tasks up to `max_attempts` times.
/// - Allows the Governor to expand the mission up to `MAX_EXPANSIONS` rounds.
/// - Streams `MissionUpdate` events through `tx` if provided (`None` = batch
///   mode; used by the CLI binary).
///
/// Returns `Ok(())` when every task completes, or an `Err` describing how
/// many tasks could not be completed.
pub async fn run_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<(), String> {
    // Wipe any context from a previous run on this plan object.
    plan.context.clear();

    let original_task_count = plan.tasks.len();
    let mut retry_attempts: u8 = 0;
    let mut expansion_rounds: u8 = 0;

    loop {
        dispatcher::dispatch_tasks(&mut plan.tasks, &mut plan.context, provider, tx.as_ref()).await;

        match governor::validate_mission(&plan.tasks, provider).await {
            governor::ValidationResult::Success => {
                finish_success(plan, original_task_count, tx.as_ref()).await;
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
                    finish_success(plan, original_task_count, tx.as_ref()).await;
                    return Ok(());
                }

                let completed_ids: Vec<uuid::Uuid> = plan.tasks.iter().map(|t| t.id).collect();

                println!("\n🔭 Governor: Expanding mission with {} new task(s) (round {}/{}).",
                    new_tasks.len(), expansion_rounds + 1, MAX_EXPANSIONS);

                if let Some(tx) = tx.as_ref() {
                    let _ = tx.send(protocol::MissionUpdate::GovernorExpand {
                        new_task_count: new_tasks.len(),
                        descriptions: new_tasks.iter().map(|t| t.description.clone()).collect(),
                    }).await;
                }

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

    if let Err(e) = persistence::save_snapshot(plan, original_task_count, "failed") {
        println!("  ⚠️  Could not save mission snapshot: {}", e);
    }

    let error = format!("{} task(s) could not be completed after {} attempts", unfinished, max_attempts);

    if let Some(tx) = tx.as_ref() {
        let _ = tx.send(protocol::MissionUpdate::MissionFailed { error: error.clone() }).await;
    }

    Err(error)
}

/// Shared success-path helper: save snapshot, emit `MissionComplete`.
async fn finish_success(
    plan: &planner::Plan,
    original_task_count: usize,
    tx: Option<&tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) {
    let layout_hint = if plan.tasks.iter().any(|t| matches!(t.role, protocol::AgentRole::Coder)) {
        "Analytical"
    } else {
        "Itinerary"
    };

    let mission_id = match persistence::save_snapshot(plan, original_task_count, "completed") {
        Ok(id) => id,
        Err(e) => {
            println!("  ⚠️  Could not save mission snapshot: {}", e);
            String::new()
        }
    };

    if let Some(tx) = tx {
        let _ = tx.send(protocol::MissionUpdate::MissionComplete {
            intent: plan.original_intent.clone(),
            task_count: plan.tasks.len(),
            expanded_task_count: plan.tasks.len().saturating_sub(original_task_count),
            mission_id,
            layout_hint: layout_hint.to_string(),
        }).await;
    }
}
