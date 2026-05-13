pub mod dispatcher;
pub mod engine;
pub mod executor;
pub mod governor;
pub mod persistence;
pub mod planner;
pub mod protocol;
pub mod registry;
pub mod tools;

/// Everything a downstream consumer needs to run a mission in one import.
///
/// ```rust
/// use axion_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::engine::{AiProvider, ImageData, MockProvider, SimpleProvider, ToolResponse};
    pub use crate::governor::{BuiltinGovernor, Governor, NewTask, ValidationResult};
    pub use crate::persistence::MissionSnapshot;
    pub use crate::planner::{build_plan_from_intent, Plan};
    pub use crate::protocol::{AgentRole, ContextBus, HandshakeRequest, MissionUpdate, Task, TaskStatus};
    pub use crate::registry::Registry;
    pub use crate::{resume_mission, run_mission};
}

const MAX_EXPANSIONS: u8 = 2;
const MAX_REFINEMENTS: u8 = 1;
const MAX_REPAIRS: u8 = 1;

/// Execute a pre-built [`Plan`] against an AI provider and a Governor.
///
/// - Retries failed tasks up to `max_attempts` times.
/// - Allows the Governor to expand the mission up to `MAX_EXPANSIONS` rounds.
/// - Streams `MissionUpdate` events through `tx` if provided (`None` = batch
///   mode; used by the CLI binary).
///
/// Returns:
/// - `Ok(None)`                     — every task completed successfully.
/// - `Ok(Some(HandshakeRequest))`   — paused awaiting human feedback; call
///                                    [`resume_mission`] with the user's reply.
/// - `Err(msg)`                     — unrecoverable failure after `max_attempts`.
pub async fn run_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    // Wipe any context from a previous run on this plan object.
    plan.context.clear();
    let original_task_count = plan.tasks.len();
    run_loop(plan, provider, governor, original_task_count, max_attempts, tx.as_ref()).await
}

/// Resume a mission that was paused by [`run_mission`] returning a
/// [`HandshakeRequest`].
///
/// `user_feedback` is injected into the [`ContextBus`] and a targeted
/// refinement task is appended to the plan so the agent can incorporate
/// the user's instructions in the next execution round.
pub async fn resume_mission(
    plan: &mut planner::Plan,
    user_feedback: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    use protocol::AWAITING_FEEDBACK_PREFIX;

    // ── 1. Strip the awaiting-feedback marker from any task that set it ───────
    let mut question = String::new();
    for task in plan.tasks.iter_mut() {
        if let Some(ref mut result) = task.result {
            if let Some(q) = result.strip_prefix(AWAITING_FEEDBACK_PREFIX) {
                // Keep only the bare question (before any "\n\nContext:" block).
                question = q.lines().next().unwrap_or(q).to_string();
                let cleaned = format!("Feedback was requested: {}", question);
                // Also update the context bus entry so context is consistent.
                plan.context.data.insert(task.slug.clone(), cleaned.clone());
                *result = cleaned;
            }
        }
    }

    // ── 2. Inject the user's response into the ContextBus ────────────────────
    plan.context
        .data
        .insert("user_feedback".to_string(), user_feedback.to_string());
    if !question.is_empty() {
        plan.context
            .data
            .insert("awaiting_feedback_question".to_string(), question.clone());
    }

    // ── 3. Append a feedback-driven refinement task ───────────────────────────
    let all_ids: Vec<uuid::Uuid> = plan.tasks.iter().map(|t| t.id).collect();
    let original_task_count = all_ids.len(); // baseline before the new task

    let refinement_intent = if question.is_empty() {
        format!(
            "Apply user feedback to improve the mission results: '{}'. \
             Review all previous context data and implement the requested changes.",
            user_feedback
        )
    } else {
        format!(
            "User has reviewed the results and responded to: '{}'. \
             Their feedback is: '{}'. \
             Review all previous context data and apply the requested changes.",
            question, user_feedback
        )
    };

    plan.add_task(
        &refinement_intent,
        all_ids,
        protocol::AgentRole::Analyst,
    );

    println!(
        "\n💬 Human feedback received — resuming mission with refinement task.\
         \n   Feedback: {}",
        user_feedback
    );

    // ── 4. Continue the loop WITHOUT clearing context ─────────────────────────
    run_loop(plan, provider, governor, original_task_count, max_attempts, tx.as_ref()).await
}

/// Core dispatch-validate loop shared by [`run_mission`] and [`resume_mission`].
///
/// Does NOT clear the context — callers are responsible for that setup step.
async fn run_loop(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    original_task_count: usize,
    max_attempts: u8,
    tx: Option<&tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    let mut retry_attempts: u8 = 0;
    let mut expansion_rounds: u8 = 0;
    let mut refinement_rounds: u8 = 0;
    let mut repair_rounds: u8 = 0;

    loop {
        dispatcher::dispatch_tasks(
            &mut plan.tasks,
            &mut plan.context,
            provider,
            governor,
            tx,
        )
        .await;

        match governor
            .validate(&plan.tasks, &plan.context, &plan.original_intent, provider)
            .await
        {
            governor::ValidationResult::Success => {
                finish_success(plan, original_task_count, tx).await;
                return Ok(None);
            }

            governor::ValidationResult::AwaitingFeedback { question } => {
                // Persist the paused state so the caller can store/resume it.
                let mission_id =
                    match persistence::save_snapshot(plan, original_task_count, "awaiting_feedback") {
                        Ok(id) => id,
                        Err(e) => {
                            println!("  ⚠️  Could not save awaiting-feedback snapshot: {}", e);
                            String::new()
                        }
                    };
                println!(
                    "\n⏸️  Mission paused — waiting for human input.\
                     \n   Question: {}",
                    question
                );
                if let Some(tx) = tx {
                    let _ = tx
                        .send(protocol::MissionUpdate::MissionPaused {
                            question: question.clone(),
                            mission_id: mission_id.clone(),
                        })
                        .await;
                }
                return Ok(Some(protocol::HandshakeRequest { mission_id, question }));
            }

            governor::ValidationResult::Retry => {
                retry_attempts += 1;

                // On the second consecutive failure, attempt dynamic re-planning
                // before falling back to a plain reset+retry.
                if retry_attempts >= 2 && repair_rounds < MAX_REPAIRS {
                    let failed: Vec<_> = plan.tasks
                        .iter()
                        .filter(|t| matches!(t.status, protocol::TaskStatus::Failed))
                        .cloned()
                        .collect();

                    if !failed.is_empty() {
                        println!(
                            "\n🔧 Re-planner: {} task(s) failed twice — consulting LLM for alternatives…",
                            failed.len()
                        );
                        let repair = planner::repair_failed_tasks(
                            &failed,
                            &plan.original_intent,
                            provider,
                        )
                        .await;

                        if !repair.is_empty() {
                            // Mark originals as superseded so the Governor no
                            // longer counts them as failures.
                            for task in plan.tasks
                                .iter_mut()
                                .filter(|t| matches!(t.status, protocol::TaskStatus::Failed))
                            {
                                task.status = protocol::TaskStatus::Completed;
                                task.result =
                                    Some("[Superseded — repair plan injected]".to_string());
                            }

                            let completed_ids: Vec<uuid::Uuid> = plan.tasks
                                .iter()
                                .filter(|t| matches!(t.status, protocol::TaskStatus::Completed))
                                .map(|t| t.id)
                                .collect();

                            if let Some(tx) = tx {
                                let _ = tx
                                    .send(protocol::MissionUpdate::GovernorExpand {
                                        new_task_count: repair.len(),
                                        descriptions: repair
                                            .iter()
                                            .map(|t| t.description.clone())
                                            .collect(),
                                    })
                                    .await;
                            }

                            for rt in repair {
                                plan.add_task_excluded(
                                    &rt.description,
                                    completed_ids.clone(),
                                    rt.role,
                                    rt.excluded_tools,
                                );
                            }

                            repair_rounds += 1;
                            retry_attempts = 0; // fresh budget for the repair tasks
                            continue;
                        }
                    }
                }

                if retry_attempts >= max_attempts {
                    break;
                }

                governor::reset_failed_tasks(&mut plan.tasks);
                println!(
                    "🚨 Failure detected. Attempting self-healing ({}/{})...",
                    retry_attempts, max_attempts
                );
            }

            governor::ValidationResult::Expand(new_tasks) => {
                if expansion_rounds >= MAX_EXPANSIONS {
                    println!(
                        "  ⚠️  Governor: Max expansions ({}) reached — treating as SUCCESS.",
                        MAX_EXPANSIONS
                    );
                    finish_success(plan, original_task_count, tx).await;
                    return Ok(None);
                }

                let completed_ids: Vec<uuid::Uuid> = plan.tasks.iter().map(|t| t.id).collect();

                println!(
                    "\n🔭 Governor: Expanding mission with {} new task(s) (round {}/{}).",
                    new_tasks.len(),
                    expansion_rounds + 1,
                    MAX_EXPANSIONS
                );

                if let Some(tx) = tx {
                    let _ = tx
                        .send(protocol::MissionUpdate::GovernorExpand {
                            new_task_count: new_tasks.len(),
                            descriptions: new_tasks.iter().map(|t| t.description.clone()).collect(),
                        })
                        .await;
                }

                for nt in new_tasks {
                    plan.add_task_excluded(&nt.description, completed_ids.clone(), nt.role, nt.excluded_tools);
                }

                expansion_rounds += 1;
            }

            governor::ValidationResult::Refine(new_tasks) => {
                if refinement_rounds >= MAX_REFINEMENTS {
                    println!(
                        "  ⚠️  Governor: Max refinements ({}) reached — treating as SUCCESS.",
                        MAX_REFINEMENTS
                    );
                    finish_success(plan, original_task_count, tx).await;
                    return Ok(None);
                }

                let completed_ids: Vec<uuid::Uuid> = plan.tasks.iter().map(|t| t.id).collect();

                println!(
                    "\n🔧 Governor: Requesting quality refinement (round {}/{}).",
                    refinement_rounds + 1,
                    MAX_REFINEMENTS
                );

                if let Some(tx) = tx {
                    let _ = tx
                        .send(protocol::MissionUpdate::GovernorExpand {
                            new_task_count: new_tasks.len(),
                            descriptions: new_tasks.iter().map(|t| t.description.clone()).collect(),
                        })
                        .await;
                }

                for nt in new_tasks {
                    plan.add_task_excluded(&nt.description, completed_ids.clone(), nt.role, nt.excluded_tools);
                }

                refinement_rounds += 1;
            }
        }
    }

    let unfinished = plan
        .tasks
        .iter()
        .filter(|t| !matches!(t.status, protocol::TaskStatus::Completed))
        .count();

    if let Err(e) = persistence::save_snapshot(plan, original_task_count, "failed") {
        println!("  ⚠️  Could not save mission snapshot: {}", e);
    }

    let error = format!(
        "{} task(s) could not be completed after {} attempts",
        unfinished, max_attempts
    );

    if let Some(tx) = tx {
        let _ = tx
            .send(protocol::MissionUpdate::MissionFailed { error: error.clone() })
            .await;
    }

    Err(error)
}

/// Extract the final [`MissionState`] from completed task results, if any.
fn extract_mission_state(tasks: &[protocol::Task]) -> Option<protocol::MissionState> {
    tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .filter_map(|r| serde_json::from_str::<protocol::MissionState>(r).ok())
        .filter(|s| !s.data_payload.is_null())
        .last()
}

/// Shared success-path helper: save snapshot, emit `MissionComplete`.
async fn finish_success(
    plan: &planner::Plan,
    original_task_count: usize,
    tx: Option<&tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) {
    let mission_state = extract_mission_state(&plan.tasks);

    let layout_hint = if mission_state.is_some() {
        "Synthesized"
    } else if plan
        .tasks
        .iter()
        .any(|t| matches!(t.role, protocol::AgentRole::Coder))
    {
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
        let _ = tx
            .send(protocol::MissionUpdate::MissionComplete {
                intent: plan.original_intent.clone(),
                task_count: plan.tasks.len(),
                expanded_task_count: plan.tasks.len().saturating_sub(original_task_count),
                mission_id,
                layout_hint: layout_hint.to_string(),
                mission_state,
            })
            .await;
    }
}
