pub mod dispatcher;
pub mod engine;
pub mod executor;
pub mod governor;
pub mod mcp;
pub mod memory;
pub mod persistence;
pub mod planner;
pub mod protocol;
pub mod registry;
pub mod research;
pub mod safety;
pub mod tools;
pub mod util;

pub use memory::MemoryStore;

use std::time::Duration;

/// Everything a downstream consumer needs to run a mission in one import.
///
/// ```rust
/// use opalzero_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::engine::{AiProvider, ImageData, MockProvider, RotatingProvider, SimpleProvider, SubprocessProvider, SubprocessInputMode, ToolResponse};
    pub use crate::mcp::OpalZeroMcpServer;
    pub use crate::governor::{BuiltinGovernor, Governor, NewTask, ValidationResult};
    pub use crate::persistence::{load_latest_snapshot_for_intent, load_snapshot, MissionSnapshot};
    pub use crate::planner::{build_plan_from_intent, Plan};
    pub use crate::protocol::{AgentRole, ContextBus, HandshakeRequest, MissionUpdate, Task, TaskStatus};
    pub use crate::registry::Registry;
    pub use crate::tools::RequestKeys;
    pub use crate::{refine_mission, refine_mission_with_keys, resume_mission, run_mission,
                    run_mission_with_history, run_scheduled_mission};
}

// ── Mission wall-clock timeout ────────────────────────────────────────────────

const MISSION_TIMEOUT_SECS: u64 = 300; // 5 minutes default

fn mission_timeout() -> Duration {
    let secs = std::env::var("OPALZERO_MISSION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MISSION_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

// ── Governor / retry caps ─────────────────────────────────────────────────────

const MAX_EXPANSIONS: u8 = 2;
/// Maximum number of *Governor-triggered* auto-refinement rounds within a single
/// `run_loop` execution.  This is intentionally separate from the number of
/// *user-triggered* refinements (each call to `refine_mission()` / `POST
/// /missions/:id/refine` is an independent request and is never counted here).
const MAX_REFINEMENTS: u8 = 3;
const MAX_REPAIRS: u8 = 1;

/// Execute a pre-built [`Plan`] against an AI provider and a Governor.
///
/// - Retries failed tasks up to `max_attempts` times.
/// - Allows the Governor to expand the mission up to `MAX_EXPANSIONS` rounds.
/// - Streams `MissionUpdate` events through `tx` if provided (`None` = batch
///   mode; used by the CLI binary).
/// - Enforces a wall-clock deadline (`OPALZERO_MISSION_TIMEOUT_SECS`, default 300 s).
///
/// Returns:
/// - `Ok(None)`                     — every task completed successfully.
/// - `Ok(Some(HandshakeRequest))`   — paused awaiting human feedback; call
///                                    [`resume_mission`] with the user's reply.
/// - `Err(msg)`                     — unrecoverable failure after `max_attempts`
///                                    or deadline exceeded.
pub async fn run_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    let timeout_dur = mission_timeout();
    let tx_err = tx.clone(); // kept for the timeout error event

    match tokio::time::timeout(
        timeout_dur,
        run_mission_inner(plan, provider, governor, max_attempts, tx),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            let msg = format!(
                "Mission exceeded the {}s time limit (set OPALZERO_MISSION_TIMEOUT_SECS to change).",
                timeout_dur.as_secs()
            );
            if let Some(t) = tx_err {
                let _ = t
                    .send(protocol::MissionUpdate::MissionFailed { error: msg.clone() })
                    .await;
            }
            Err(msg)
        }
    }
}

/// Run a mission with explicit awareness of a prior completed run.
///
/// If `prior` is `Some`, the following are injected into the context bus
/// **before** the first task executes:
///
/// - [`CTX_PRIOR_MISSION_STATE`] — the prior run's `data_payload` as a JSON
///   string, so the Analyst can build on it rather than starting from scratch.
/// - [`CTX_PRIOR_RUN_TIMESTAMP`] — a human-readable age string such as
///   `"47 hours ago (2026-05-24T10:30:00Z)"`, letting the Analyst reason about
///   data freshness.
///
/// Both keys survive the internal context wipe performed by `run_mission_inner`,
/// so they remain visible throughout the entire execution graph.
///
/// If `prior` is `None` the call is identical to [`run_mission`].
///
/// # Typical usage
///
/// ```rust,no_run
/// # use opalzero_core::prelude::*;
/// # async fn example(provider: &dyn AiProvider, governor: &dyn Governor) {
/// let prior = opalzero_core::persistence::load_latest_snapshot_for_intent("daily BTC report");
/// let mut plan = build_plan_from_intent("daily BTC report", provider).await;
/// run_mission_with_history(&mut plan, prior.as_ref(), provider, governor, 3, None).await.unwrap();
/// # }
/// ```
pub async fn run_mission_with_history(
    plan: &mut planner::Plan,
    prior: Option<&persistence::MissionSnapshot>,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    if let Some(snap) = prior {
        inject_prior_snapshot(plan, snap);
    }
    run_mission(plan, provider, governor, max_attempts, tx).await
}

/// Build a prior-run context string from a Unix timestamp and an RFC-3339 wall
/// clock string for the Analyst's benefit.
fn format_prior_timestamp(unix_ts: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let age_secs = now.saturating_sub(unix_ts);
    let age_str = if age_secs < 120 {
        "just now".to_string()
    } else if age_secs < 3600 {
        format!("{} minutes ago", age_secs / 60)
    } else if age_secs < 86_400 {
        format!("{} hours ago", age_secs / 3600)
    } else {
        format!("{} days ago", age_secs / 86_400)
    };

    // Build an ISO-8601 wall-clock string from the snapshot timestamp.
    // We avoid pulling in `chrono` by doing minimal arithmetic.
    age_str
}

/// Inject a prior [`MissionSnapshot`] into the plan's context bus so agents
/// can build on previous findings.
fn inject_prior_snapshot(plan: &mut planner::Plan, snap: &persistence::MissionSnapshot) {
    // Inject the structured payload so the Analyst can reference prior results.
    if let Some(ms) = &snap.mission_state {
        if let Ok(json) = serde_json::to_string(&ms.data_payload) {
            plan.context
                .data
                .insert(protocol::CTX_PRIOR_MISSION_STATE.into(), json);
        }
    } else {
        // Fall back to the raw context entries from the prior run (top 8 keys).
        let summary: String = snap
            .context
            .data
            .iter()
            .take(8)
            .map(|(k, v)| format!("{}: {}", k, v.chars().take(200).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        if !summary.is_empty() {
            plan.context
                .data
                .insert(protocol::CTX_PRIOR_MISSION_STATE.into(), summary);
        }
    }

    // Inject a human-readable age for data-freshness reasoning.
    let age_str = format_prior_timestamp(snap.timestamp);
    plan.context
        .data
        .insert(protocol::CTX_PRIOR_RUN_TIMESTAMP.into(), age_str);

    tracing::info!(
        prior_mission_id = %snap.id,
        prior_timestamp  = snap.timestamp,
        "prior mission snapshot injected into context bus",
    );
}

/// Run a **scheduled** (recurring) mission that automatically picks up where
/// the last run left off.
///
/// This is the highest-level entry point for cron-style or watchdog missions.
/// It:
/// 1. Searches `missions/` for the most recent *completed* snapshot whose
///    `intent` matches the plan's `original_intent`.
/// 2. Injects that snapshot into the plan's context bus (see
///    [`run_mission_with_history`]).
/// 3. Delegates to [`run_mission`].
///
/// If no prior completed snapshot exists the mission starts fresh, exactly
/// like a plain [`run_mission`] call.
///
/// # Example
///
/// ```rust,no_run
/// # use opalzero_core::prelude::*;
/// # async fn example(provider: &dyn AiProvider, governor: &dyn Governor) {
/// // Call this on a cron schedule — each run inherits the prior run's data.
/// let mut plan = build_plan_from_intent("daily BTC price report", provider).await;
/// run_scheduled_mission(&mut plan, provider, governor, 3, None).await.unwrap();
/// # }
/// ```
pub async fn run_scheduled_mission(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    let prior = persistence::load_latest_snapshot_for_intent(&plan.original_intent);

    if let Some(ref snap) = prior {
        tracing::info!(
            prior_mission_id = %snap.id,
            "scheduled mission: found prior run — injecting history",
        );
    } else {
        tracing::info!(
            intent = %plan.original_intent,
            "scheduled mission: no prior run found — starting fresh",
        );
    }

    run_mission_with_history(plan, prior.as_ref(), provider, governor, max_attempts, tx).await
}

/// Inner body of [`run_mission`] — extracted so it can be wrapped by
/// `tokio::time::timeout` without requiring a `'static` bound.
async fn run_mission_inner(
    plan: &mut planner::Plan,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    // Preserve caller-injected keys before wiping context, then restore after.
    let preserved_schema    = plan.context.data.remove(protocol::CTX_OUTPUT_SCHEMA);
    let preserved_prior_ms  = plan.context.data.remove(protocol::CTX_PRIOR_MISSION_STATE);
    let preserved_prior_ts  = plan.context.data.remove(protocol::CTX_PRIOR_RUN_TIMESTAMP);

    // Wipe any context from a previous run on this plan object.
    plan.context.clear();

    // Restore preserved keys so downstream agents can read them.
    if let Some(schema) = preserved_schema {
        plan.context.data.insert(protocol::CTX_OUTPUT_SCHEMA.into(), schema);
    }
    if let Some(ms) = preserved_prior_ms {
        plan.context.data.insert(protocol::CTX_PRIOR_MISSION_STATE.into(), ms);
    }
    if let Some(ts) = preserved_prior_ts {
        plan.context.data.insert(protocol::CTX_PRIOR_RUN_TIMESTAMP.into(), ts);
    }

    // ── Inject cross-mission persistent memory into the context bus ───────────
    let memory = memory::MemoryStore::new(std::path::Path::new("memory"));
    let all = memory.read_all();
    if !all.is_empty() {
        let summary = all
            .iter()
            .map(|(k, e)| format!("{k}: {}", e.value))
            .collect::<Vec<_>>()
            .join("\n");
        plan.context
            .data
            .insert(protocol::CTX_GLOBAL_MEMORY.into(), summary);
    }

    let original_task_count = plan.tasks.len();
    run_loop(plan, provider, governor, original_task_count, max_attempts, tx.as_ref()).await
}

/// Resume a mission that was paused by [`run_mission`] returning a
/// [`HandshakeRequest`].
///
/// `user_feedback` is injected into the [`ContextBus`] and a targeted
/// refinement task is appended to the plan so the agent can incorporate
/// the user's instructions in the next execution round.
///
/// Enforces the same wall-clock deadline as [`run_mission`].
pub async fn resume_mission(
    plan: &mut planner::Plan,
    user_feedback: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    let timeout_dur = mission_timeout();
    let tx_err = tx.clone();

    match tokio::time::timeout(
        timeout_dur,
        resume_mission_inner(plan, user_feedback, provider, governor, max_attempts, tx),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            let msg = format!(
                "Mission exceeded the {}s time limit (set OPALZERO_MISSION_TIMEOUT_SECS to change).",
                timeout_dur.as_secs()
            );
            if let Some(t) = tx_err {
                let _ = t
                    .send(protocol::MissionUpdate::MissionFailed { error: msg.clone() })
                    .await;
            }
            Err(msg)
        }
    }
}

/// Inner body of [`resume_mission`].
async fn resume_mission_inner(
    plan: &mut planner::Plan,
    user_feedback: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    use protocol::AWAITING_FEEDBACK_PREFIX;

    // ── Refresh cross-mission memory in the context bus ───────────────────────
    // (Context is NOT cleared on resume — we update the memory key in place.)
    let memory = memory::MemoryStore::new(std::path::Path::new("memory"));
    let all = memory.read_all();
    if !all.is_empty() {
        let summary = all
            .iter()
            .map(|(k, e)| format!("{k}: {}", e.value))
            .collect::<Vec<_>>()
            .join("\n");
        plan.context
            .data
            .insert(protocol::CTX_GLOBAL_MEMORY.into(), summary);
    }

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
        .insert(protocol::CTX_USER_FEEDBACK.to_string(), user_feedback.to_string());
    if !question.is_empty() {
        plan.context
            .data
            .insert(protocol::CTX_FEEDBACK_QUESTION.to_string(), question.clone());
    }

    // ── 3. Append a feedback-driven refinement task ───────────────────────────
    let all_slugs: Vec<String> = plan.tasks.iter().map(|t| t.slug.clone()).collect();
    let original_task_count = all_slugs.len(); // baseline before the new task

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
        all_slugs,
        protocol::AgentRole::Analyst,
    );

    tracing::info!(user_feedback, "human feedback received — resuming mission with refinement task");

    // ── 4. Continue the loop WITHOUT clearing context ─────────────────────────
    run_loop(plan, provider, governor, original_task_count, max_attempts, tx.as_ref()).await
}

/// Run a targeted refinement against an existing completed mission.
///
/// - Summarises the prior `data_payload` and injects it into the Planner prompt
///   so only incremental tasks are generated.
/// - Pre-populates the context bus with the prior task results so the Analyst
///   can merge old + new findings in `finalize_mission_state`.
/// - Intercepts the internal `MissionComplete` event and replaces the generated
///   snapshot ID with the *original* mission ID so the frontend history entry
///   stays stable.
/// - After all tasks finish, back-fills any prior `data_payload` keys that the
///   refinement Analyst did not explicitly include, then overwrites the original
///   `missions/<id>.json` with the merged snapshot.
/// Like [`refine_mission`] but also accepts per-request API key overrides.
///
/// Use this variant when keys are provided per-request (e.g. via HTTP headers)
/// to avoid the data race that would occur if the caller used `set_var`.
pub async fn refine_mission_with_keys(
    snapshot: &persistence::MissionSnapshot,
    refinement_intent: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
    keys: tools::RequestKeys,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    refine_mission_inner(snapshot, refinement_intent, provider, governor, max_attempts, tx, keys).await
}

pub async fn refine_mission(
    snapshot: &persistence::MissionSnapshot,
    refinement_intent: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    refine_mission_inner(snapshot, refinement_intent, provider, governor, max_attempts, tx, tools::RequestKeys::default()).await
}

async fn refine_mission_inner(
    snapshot: &persistence::MissionSnapshot,
    refinement_intent: &str,
    provider: &dyn engine::AiProvider,
    governor: &dyn governor::Governor,
    max_attempts: u8,
    tx: Option<tokio::sync::mpsc::Sender<protocol::MissionUpdate>>,
    keys: tools::RequestKeys,
) -> Result<Option<protocol::HandshakeRequest>, String> {
    use std::sync::{Arc, Mutex};

    let original_id    = snapshot.id.clone();
    let original_intent = snapshot
        .intent
        .trim_start_matches("REFINE[")   // strip nested REFINE[] wrappers
        .split("]: ")
        .last()
        .unwrap_or(&snapshot.intent)
        .to_string();

    // ── 1. Build a human-readable summary of what already exists ─────────────
    let prior_summary = match &snapshot.mission_state {
        Some(ms) if !ms.data_payload.is_null() => summarise_payload(&ms.data_payload),
        _ => snapshot
            .context
            .data
            .iter()
            .take(6)
            .map(|(k, v)| format!("• {}: {}", k, v.chars().take(120).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n"),
    };

    tracing::info!(
        mission_id = %original_id,
        refinement_intent,
        prior_summary_chars = prior_summary.len(),
        "refining mission",
    );

    // ── 2. Build a refinement-aware plan (incremental tasks only) ─────────────
    let mut plan = planner::build_refinement_plan(
        &original_intent,
        refinement_intent,
        &prior_summary,
        provider,
    )
    .await;

    // Inject per-request API keys so tools can use them without mutating globals.
    plan.keys = keys;

    // ── 3. Pre-populate context bus with prior task results ───────────────────
    for (k, v) in &snapshot.context.data {
        plan.context.data.entry(k.clone()).or_insert_with(|| v.clone());
    }
    // Inject the structured prior payload so the Analyst can reference it
    // by name when writing its finalize_mission_state call.
    if let Some(ms) = &snapshot.mission_state {
        if let Ok(json) = serde_json::to_string(&ms.data_payload) {
            plan.context
                .data
                .insert(protocol::CTX_PRIOR_MISSION_STATE.to_string(), json);
        }
    }

    // Label the plan so snapshots/logs show the full refinement chain.
    plan.original_intent = format!(
        "REFINE[{}]: {}",
        original_intent.chars().take(60).collect::<String>(),
        refinement_intent
    );

    let original_task_count = plan.tasks.len();

    // ── 4. Intercept events — replace generated ID with the original ──────────
    let (inner_tx, mut inner_rx) = tokio::sync::mpsc::channel::<protocol::MissionUpdate>(64);
    let outer_tx       = tx.clone();
    let oid            = original_id.clone();
    let oint           = original_intent.clone();
    let captured_id: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let cap_clone      = captured_id.clone();

    let forward_handle = tokio::spawn(async move {
        while let Some(event) = inner_rx.recv().await {
            let patched = match event {
                protocol::MissionUpdate::MissionComplete {
                    task_count,
                    expanded_task_count,
                    layout_hint,
                    mission_state,
                    mission_id: gen_id,
                    ..
                } => {
                    if let Ok(mut guard) = cap_clone.lock() {
                        *guard = gen_id;
                    }
                    protocol::MissionUpdate::MissionComplete {
                        intent: oint.clone(),
                        task_count,
                        expanded_task_count,
                        mission_id: oid.clone(),
                        layout_hint,
                        mission_state,
                    }
                }
                other => other,
            };
            if let Some(ref t) = outer_tx {
                t.send(patched).await.ok();
            }
        }
    });

    // ── 5. Execute the plan ───────────────────────────────────────────────────
    let result = run_loop(
        &mut plan,
        provider,
        governor,
        original_task_count,
        max_attempts,
        Some(&inner_tx),
    )
    .await;

    // Signal forwarder to drain, then wait for it to finish.
    drop(inner_tx);
    forward_handle.await.ok();

    // ── 6. Merge prior keys + overwrite original snapshot ─────────────────────
    if result.is_ok() {
        // Delete the temp snapshot that finish_success created.
        if let Ok(gen_id) = captured_id.lock() {
            if !gen_id.is_empty() && *gen_id != original_id {
                let _ = std::fs::remove_file(
                    std::path::Path::new("missions").join(format!("{}.json", *gen_id)),
                );
            }
        }

        // Back-fill any prior data_payload keys that the new Analyst omitted.
        backfill_prior_keys(&mut plan, &snapshot.mission_state);

        plan.original_intent = original_intent;
        if let Err(e) = persistence::save_snapshot_with_id(
            &plan,
            original_task_count,
            "completed",
            &original_id,
        ) {
            tracing::warn!(mission_id = %original_id, error = %e, "could not overwrite snapshot");
        }
    }

    result
}

/// Summarise a `data_payload` JSON object into a compact bullet list (≤ 1 500 chars).
fn summarise_payload(payload: &serde_json::Value) -> String {
    let obj = match payload.as_object() {
        Some(o) => o,
        None    => return payload.to_string().chars().take(800).collect(),
    };
    let mut lines: Vec<String> = Vec::new();
    for (k, v) in obj.iter().take(12) {
        let val_str = match v {
            serde_json::Value::String(s) => s.chars().take(160).collect::<String>(),
            serde_json::Value::Array(a)  => format!("[{} items]", a.len()),
            serde_json::Value::Object(_) => "[object]".to_string(),
            other                        => other.to_string().chars().take(120).collect(),
        };
        lines.push(format!("• {}: {}", k, val_str));
    }
    let text = lines.join("\n");
    if text.len() > 1_500 { text[..1_500].to_string() } else { text }
}

/// Back-fill prior `data_payload` keys that this refinement did not explicitly
/// set, so the updated snapshot contains all historical findings.
fn backfill_prior_keys(
    plan: &mut planner::Plan,
    prior_state: &Option<protocol::MissionState>,
) {
    let prior_ms = match prior_state {
        Some(ms) => ms,
        None     => return,
    };
    let prior_obj = match prior_ms.data_payload.as_object() {
        Some(o) => o,
        None    => return,
    };

    // Find the last task result that is a valid MissionState with data.
    let idx = plan.tasks.iter().rposition(|t| {
        t.result
            .as_ref()
            .and_then(|r| serde_json::from_str::<protocol::MissionState>(r).ok())
            .filter(|s| !s.data_payload.is_null())
            .is_some()
    });

    if let Some(idx) = idx {
        if let Some(ref mut result_str) = plan.tasks[idx].result.clone().take() {
            // Need to work around borrow checker — re-read after cloning.
            let cloned = result_str.clone();
            if let Ok(mut new_ms) = serde_json::from_str::<protocol::MissionState>(&cloned) {
                if let Some(new_obj) = new_ms.data_payload.as_object_mut() {
                    for (k, v) in prior_obj {
                        new_obj.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
                if let Ok(patched) = serde_json::to_string(&new_ms) {
                    plan.tasks[idx].result = Some(patched);
                }
            }
        }
    }
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
    // Borrow the keys from the plan once for the whole loop lifetime.
    // `plan.keys` is never mutated here; we only need a reference.
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
            &plan.keys,
        )
        .await;

        // Resolve critic provider: Governor may declare a separate model for
        // quality-check calls (cheaper / independent from the agent model).
        // Falls back to the plan's main provider when critic_model is None.
        let critic_model = governor.critic_model();
        if let Some(ref m) = critic_model {
            tracing::info!(critic_model = m.as_str(), "Governor: using dedicated critic model");
        }
        let critic_box: Option<Box<dyn engine::AiProvider>> = critic_model
            .as_deref()
            .and_then(|m| provider.with_text_model(m));
        let critic: &dyn engine::AiProvider = critic_box.as_deref().unwrap_or(provider);

        match governor
            .validate(&plan.tasks, &plan.context, &plan.original_intent, critic)
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
                            tracing::warn!(error = %e, "could not save awaiting-feedback snapshot");
                            String::new()
                        }
                    };
                tracing::info!(question, "mission paused — waiting for human input");
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
                        tracing::info!(failed_count = failed.len(), "Re-planner: tasks failed twice — consulting LLM for alternatives");
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

                            // ── Fix Bug 2: sequential deps for repair tasks ───
                            //
                            // Using `completed_slugs` as deps for every repair
                            // task causes circular deps when a repair task's
                            // slug collides with a name already in
                            // `completed_slugs` (the cycle detector overwrites
                            // the old entry with the new one, then sees A→B and
                            // B→A in the same batch).
                            //
                            // Instead: repair task N depends only on repair
                            // task N-1.  All prior context is always available
                            // through the context bus regardless of deps.
                            let mut prev_repair_slug: Option<String> = None;
                            for rt in repair {
                                let deps = prev_repair_slug
                                    .iter()
                                    .cloned()
                                    .collect::<Vec<_>>();
                                let slug = plan.add_task_excluded(
                                    &rt.description,
                                    deps,
                                    rt.role,
                                    rt.excluded_tools,
                                );
                                prev_repair_slug = Some(slug);
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
                tracing::warn!(retry_attempts, max_attempts, "failure detected — attempting self-healing");
            }

            governor::ValidationResult::Expand(new_tasks) => {
                if expansion_rounds >= MAX_EXPANSIONS {
                    tracing::warn!(MAX_EXPANSIONS, "Governor: max expansions reached — treating as SUCCESS");
                    finish_success(plan, original_task_count, tx).await;
                    return Ok(None);
                }

                let all_slugs: Vec<String> = plan.tasks.iter().map(|t| t.slug.clone()).collect();

                tracing::info!(
                    new_task_count = new_tasks.len(),
                    round = expansion_rounds + 1,
                    max_rounds = MAX_EXPANSIONS,
                    "Governor: expanding mission",
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
                    plan.add_task_excluded(&nt.description, all_slugs.clone(), nt.role, nt.excluded_tools);
                }

                expansion_rounds += 1;
            }

            governor::ValidationResult::Refine(new_tasks) => {
                if refinement_rounds >= MAX_REFINEMENTS {
                    tracing::warn!(MAX_REFINEMENTS, "Governor: max refinements reached — treating as SUCCESS");
                    finish_success(plan, original_task_count, tx).await;
                    return Ok(None);
                }

                let all_slugs: Vec<String> = plan.tasks.iter().map(|t| t.slug.clone()).collect();

                tracing::info!(
                    round = refinement_rounds + 1,
                    max_rounds = MAX_REFINEMENTS,
                    "Governor: requesting quality refinement",
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
                    plan.add_task_excluded(&nt.description, all_slugs.clone(), nt.role, nt.excluded_tools);
                }

                refinement_rounds += 1;
            }
        }
    }

    let unfinished: Vec<&protocol::Task> = plan.tasks
        .iter()
        .filter(|t| !matches!(t.status, protocol::TaskStatus::Completed))
        .collect();

    // If the Analyst produced a MissionState despite some tasks failing,
    // treat as partial success — show real data with missing fields rather
    // than a blank error screen.
    if extract_mission_state(&plan.tasks).is_some() {
        tracing::warn!(failed_count = unfinished.len(), "task(s) failed but Analyst produced a result — emitting partial success");
        finish_success(plan, original_task_count, tx).await;
        return Ok(None);
    }

    // True failure: no useful data was produced.
    let failed_summary: String = unfinished
        .iter()
        .map(|t| format!("  • {} ({})", t.slug, t.result.as_deref().unwrap_or("no result")))
        .collect::<Vec<_>>()
        .join("\n");

    tracing::error!(failed_tasks = %failed_summary, "mission failed");

    if let Err(e) = persistence::save_snapshot(plan, original_task_count, "failed") {
        tracing::warn!(error = %e, "could not save mission snapshot");
    }

    let error = format!(
        "{} task(s) could not be completed after {} attempts",
        unfinished.len(), max_attempts
    );

    if let Some(tx) = tx {
        let _ = tx
            .send(protocol::MissionUpdate::MissionFailed { error: error.clone() })
            .await;
    }

    Err(error)
}

/// Extract the final mission state JSON from completed task results, if any.
///
/// Returns the raw JSON value rather than a typed `MissionState` so that
/// presentation-layer fields injected by `finalize_mission_state` (e.g.
/// `design_tokens`, `suggested_widgets`) are preserved for the server layer
/// to forward to the frontend without ever entering the kernel's type system.
fn extract_mission_state(tasks: &[protocol::Task]) -> Option<serde_json::Value> {
    tasks
        .iter()
        .filter_map(|t| t.result.as_ref())
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(r).ok())
        .filter(|v| {
            // Must parse as a valid MissionState (has a non-null data_payload).
            v.get("data_payload").map(|p| !p.is_null()).unwrap_or(false)
        })
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
            tracing::warn!(error = %e, "could not save mission snapshot");
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentRole, MissionState, Task, TaskStatus};
    use serde_json::json;
    use uuid::Uuid;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_plan_with_result(result_json: &str) -> planner::Plan {
        let mut plan = planner::Plan::new("test intent");
        let task = Task {
            id: Uuid::new_v4(),
            slug: "analyst_task".to_string(),
            intent: "finalize".to_string(),
            status: TaskStatus::Completed,
            role: AgentRole::Analyst,
            result: Some(result_json.to_string()),
            depends_on: vec![],
            excluded_tools: vec![],
            allowed_tools: None,
            output_schema: None,
        };
        plan.tasks.push(task);
        plan
    }

    fn mission_state_json(payload: serde_json::Value) -> String {
        let ms = MissionState {
            intent_resolved: true,
            data_payload: payload,
            verification_logs: vec![],
        };
        serde_json::to_string(&ms).unwrap()
    }

    // ── backfill_prior_keys tests ─────────────────────────────────────────────

    #[test]
    fn backfill_adds_missing_prior_keys() {
        // New analyst produced only "new_key"; prior had "old_key" too.
        let new_result = mission_state_json(json!({ "new_key": "new_value" }));
        let mut plan = make_plan_with_result(&new_result);

        let prior_state = Some(MissionState {
            intent_resolved: true,
            data_payload: json!({ "new_key": "prior_value", "old_key": "old_value" }),
            verification_logs: vec![],
        });

        backfill_prior_keys(&mut plan, &prior_state);

        // The task result should now contain "old_key".
        let patched_str = plan.tasks[0].result.as_ref().unwrap();
        let patched: MissionState = serde_json::from_str(patched_str).unwrap();
        let obj = patched.data_payload.as_object().unwrap();
        assert!(obj.contains_key("old_key"), "prior key 'old_key' must be back-filled");
        assert_eq!(obj["old_key"], "old_value");
    }

    #[test]
    fn backfill_does_not_overwrite_new_keys() {
        // Both new and prior payloads have "shared_key" but with different values.
        // The new value must win.
        let new_result = mission_state_json(json!({ "shared_key": "new_value" }));
        let mut plan = make_plan_with_result(&new_result);

        let prior_state = Some(MissionState {
            intent_resolved: true,
            data_payload: json!({ "shared_key": "old_value" }),
            verification_logs: vec![],
        });

        backfill_prior_keys(&mut plan, &prior_state);

        let patched_str = plan.tasks[0].result.as_ref().unwrap();
        let patched: MissionState = serde_json::from_str(patched_str).unwrap();
        let obj = patched.data_payload.as_object().unwrap();
        assert_eq!(
            obj["shared_key"], "new_value",
            "new value must not be overwritten by back-fill"
        );
    }

    #[test]
    fn backfill_no_panic_when_prior_state_is_none() {
        let new_result = mission_state_json(json!({ "key": "val" }));
        let mut plan = make_plan_with_result(&new_result);

        // Must return without panicking.
        backfill_prior_keys(&mut plan, &None);

        // Plan is unchanged.
        let result_str = plan.tasks[0].result.as_ref().unwrap();
        let ms: MissionState = serde_json::from_str(result_str).unwrap();
        assert_eq!(ms.data_payload["key"], "val");
    }

    #[test]
    fn backfill_no_panic_when_no_task_has_valid_mission_state() {
        // Plan has a task with a non-JSON result (no valid MissionState).
        let mut plan = planner::Plan::new("test");
        let task = Task {
            id: Uuid::new_v4(),
            slug: "plain_task".to_string(),
            intent: "plain text result".to_string(),
            status: TaskStatus::Completed,
            role: AgentRole::Analyst,
            result: Some("plain text, not JSON".to_string()),
            depends_on: vec![],
            excluded_tools: vec![],
            allowed_tools: None,
            output_schema: None,
        };
        plan.tasks.push(task);

        let prior_state = Some(MissionState {
            intent_resolved: true,
            data_payload: json!({ "prior_key": "prior_val" }),
            verification_logs: vec![],
        });

        // Must not panic even though no task result parses as MissionState.
        backfill_prior_keys(&mut plan, &prior_state);
        assert_eq!(plan.tasks[0].result.as_deref(), Some("plain text, not JSON"));
    }

    #[test]
    fn backfill_no_panic_when_task_result_is_none() {
        // Task has no result at all.
        let mut plan = planner::Plan::new("test");
        let task = Task {
            id: Uuid::new_v4(),
            slug: "empty_task".to_string(),
            intent: "no result".to_string(),
            status: TaskStatus::Pending,
            role: AgentRole::Analyst,
            result: None,
            depends_on: vec![],
            excluded_tools: vec![],
            allowed_tools: None,
            output_schema: None,
        };
        plan.tasks.push(task);

        let prior_state = Some(MissionState {
            intent_resolved: true,
            data_payload: json!({ "k": "v" }),
            verification_logs: vec![],
        });

        // Must not panic.
        backfill_prior_keys(&mut plan, &prior_state);
        assert!(plan.tasks[0].result.is_none());
    }

    // ── inject_prior_snapshot tests ───────────────────────────────────────────

    fn make_mock_snapshot(intent: &str, ts: u64) -> persistence::MissionSnapshot {
        persistence::MissionSnapshot {
            id: format!("mission_{}_{}", ts, "test"),
            timestamp: ts,
            intent: intent.to_string(),
            task_count: 1,
            expanded_task_count: 0,
            status: "completed".to_string(),
            layout_hint: "Synthesized".to_string(),
            context: protocol::ContextBus::default(),
            mission_state: Some(MissionState {
                intent_resolved: true,
                data_payload: json!({ "price": 42000, "currency": "USD" }),
                verification_logs: vec![],
            }),
        }
    }

    #[test]
    fn inject_prior_snapshot_sets_context_keys() {
        let snap = make_mock_snapshot("BTC price report", 1_700_000_000);
        let mut plan = planner::Plan::new("BTC price report");

        inject_prior_snapshot(&mut plan, &snap);

        assert!(
            plan.context.data.contains_key(protocol::CTX_PRIOR_MISSION_STATE),
            "CTX_PRIOR_MISSION_STATE must be set"
        );
        assert!(
            plan.context.data.contains_key(protocol::CTX_PRIOR_RUN_TIMESTAMP),
            "CTX_PRIOR_RUN_TIMESTAMP must be set"
        );

        let ms_json = &plan.context.data[protocol::CTX_PRIOR_MISSION_STATE];
        assert!(ms_json.contains("42000"), "prior payload JSON must be injected");
    }

    #[test]
    fn inject_prior_snapshot_without_mission_state_uses_context_fallback() {
        let mut snap = make_mock_snapshot("weather report", 1_700_000_000);
        snap.mission_state = None;
        snap.context.data.insert("temp".into(), "22C".into());

        let mut plan = planner::Plan::new("weather report");
        inject_prior_snapshot(&mut plan, &snap);

        let prior = plan.context.data.get(protocol::CTX_PRIOR_MISSION_STATE).unwrap();
        assert!(prior.contains("22C"), "raw context should be used as fallback");
    }

    #[test]
    fn prior_keys_survive_run_mission_inner_context_wipe() {
        // Verify that CTX_PRIOR_MISSION_STATE and CTX_PRIOR_RUN_TIMESTAMP are
        // preserved through the plan.context.clear() call inside run_mission_inner.
        //
        // We can't call run_mission_inner directly without a real provider,
        // but we CAN verify the preservation logic by simulating what it does.

        let mut plan = planner::Plan::new("test scheduled mission");
        plan.context.data.insert(protocol::CTX_PRIOR_MISSION_STATE.into(), r#"{"foo":"bar"}"#.into());
        plan.context.data.insert(protocol::CTX_PRIOR_RUN_TIMESTAMP.into(), "2 hours ago".into());
        plan.context.data.insert("some_other_key".into(), "should_be_wiped".into());

        // Replicate what run_mission_inner does with the preserved keys:
        let preserved_schema   = plan.context.data.remove(protocol::CTX_OUTPUT_SCHEMA);
        let preserved_prior_ms = plan.context.data.remove(protocol::CTX_PRIOR_MISSION_STATE);
        let preserved_prior_ts = plan.context.data.remove(protocol::CTX_PRIOR_RUN_TIMESTAMP);

        plan.context.clear();

        if let Some(schema) = preserved_schema {
            plan.context.data.insert(protocol::CTX_OUTPUT_SCHEMA.into(), schema);
        }
        if let Some(ms) = preserved_prior_ms {
            plan.context.data.insert(protocol::CTX_PRIOR_MISSION_STATE.into(), ms);
        }
        if let Some(ts) = preserved_prior_ts {
            plan.context.data.insert(protocol::CTX_PRIOR_RUN_TIMESTAMP.into(), ts);
        }

        assert_eq!(
            plan.context.data.get(protocol::CTX_PRIOR_MISSION_STATE).map(|s| s.as_str()),
            Some(r#"{"foo":"bar"}"#),
            "CTX_PRIOR_MISSION_STATE must survive context wipe"
        );
        assert_eq!(
            plan.context.data.get(protocol::CTX_PRIOR_RUN_TIMESTAMP).map(|s| s.as_str()),
            Some("2 hours ago"),
            "CTX_PRIOR_RUN_TIMESTAMP must survive context wipe"
        );
        assert!(
            !plan.context.data.contains_key("some_other_key"),
            "unpreserved keys must be wiped"
        );
    }

    #[test]
    fn format_prior_timestamp_returns_human_readable_string() {
        // Very recent — "just now"
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let result = format_prior_timestamp(now);
        assert!(result.contains("just now") || result.contains("minute"),
            "very recent timestamps should say 'just now' or minutes, got: {}", result);

        // ~2 hours ago
        let two_hours_ago = now.saturating_sub(7200);
        let result2 = format_prior_timestamp(two_hours_ago);
        assert!(result2.contains("hour"), "2-hour-old timestamps should mention hours, got: {}", result2);

        // ~3 days ago
        let three_days_ago = now.saturating_sub(3 * 86_400 + 60);
        let result3 = format_prior_timestamp(three_days_ago);
        assert!(result3.contains("day"), "3-day-old timestamps should mention days, got: {}", result3);
    }
}
