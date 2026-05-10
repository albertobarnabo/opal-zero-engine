use async_trait::async_trait;
use axion_core::engine::{AiProvider, ToolResponse};
use axion_core::executor::wasm::WasmExecutor;
use axion_core::planner::Plan;
use axion_core::protocol::{AgentRole, MissionUpdate, Tool};
use axion_core::run_mission;

// ── MockProvider ─────────────────────────────────────────────────────────────

struct MockProvider;

#[async_trait]
impl AiProvider for MockProvider {
    async fn generate_response(
        &self,
        _prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        if tools.is_none() {
            // Governor call — always approve.
            Ok(ToolResponse::Text(
                r#"{"verdict":"SUCCESS","reasoning":"mock approval","new_tasks":[]}"#.to_string(),
            ))
        } else {
            Ok(ToolResponse::Text("mock result".to_string()))
        }
    }

    async fn submit_tool_result(
        &self,
        _prompt: &str,
        _tools: Option<Vec<Tool>>,
        _tool_call_id: &str,
        _tool_name: &str,
        _tool_arguments: &str,
        _tool_result: &str,
    ) -> Result<ToolResponse, String> {
        Ok(ToolResponse::Text("mock result after tool".to_string()))
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Collect all events from `rx` until a terminal event arrives or the channel closes.
async fn drain(
    mut rx: tokio::sync::mpsc::Receiver<MissionUpdate>,
) -> Vec<MissionUpdate> {
    let mut events = vec![];
    while let Some(event) = rx.recv().await {
        let done = matches!(
            &event,
            MissionUpdate::MissionComplete { .. } | MissionUpdate::MissionFailed { .. }
        );
        events.push(event);
        if done {
            break;
        }
    }
    events
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// The event stream for a single-task plan must follow the order:
/// TaskStarted → TaskCompleted → MissionComplete.
#[tokio::test]
async fn event_stream_order_started_completed_mission_complete() {
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(32);

    let mut plan = Plan::new("Single task test plan");
    plan.add_task("Do one thing", vec![], AgentRole::Analyst);

    tokio::spawn(async move {
        let provider = MockProvider;
        let _ = run_mission(&mut plan, &provider, 3, Some(tx)).await;
    });

    let events = drain(rx).await;

    // All three required event types must be present.
    let started_idx = events
        .iter()
        .position(|e| matches!(e, MissionUpdate::TaskStarted { .. }))
        .expect("Expected at least one TaskStarted event");

    let completed_idx = events
        .iter()
        .position(|e| matches!(e, MissionUpdate::TaskCompleted { .. }))
        .expect("Expected at least one TaskCompleted event");

    let mission_idx = events
        .iter()
        .position(|e| matches!(e, MissionUpdate::MissionComplete { .. }))
        .expect("Expected MissionComplete event");

    // Ordering contract.
    assert!(
        started_idx < completed_idx,
        "TaskStarted ({}) must precede TaskCompleted ({})",
        started_idx,
        completed_idx
    );
    assert!(
        completed_idx < mission_idx,
        "TaskCompleted ({}) must precede MissionComplete ({})",
        completed_idx,
        mission_idx
    );
}

/// MissionComplete must carry the correct task count and a non-empty mission_id.
#[tokio::test]
async fn mission_complete_carries_correct_metadata() {
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(32);

    let mut plan = Plan::new("Metadata test");
    plan.add_task("Task one", vec![], AgentRole::WebSearcher);
    plan.add_task("Task two", vec![], AgentRole::Analyst);

    tokio::spawn(async move {
        let provider = MockProvider;
        let _ = run_mission(&mut plan, &provider, 3, Some(tx)).await;
    });

    let events = drain(rx).await;

    let complete = events
        .iter()
        .find_map(|e| {
            if let MissionUpdate::MissionComplete {
                task_count,
                mission_id,
                ..
            } = e
            {
                Some((*task_count, mission_id.clone()))
            } else {
                None
            }
        })
        .expect("MissionComplete event not found");

    let (task_count, mission_id) = complete;
    assert_eq!(task_count, 2, "task_count should equal the number of tasks in the plan");
    assert!(
        mission_id.starts_with("mission_"),
        "mission_id should start with 'mission_', got: {}",
        mission_id
    );
}

/// run_mission must clear the ContextBus at the start so stale data from a
/// previous run (or manually injected data) never leaks into the new mission.
#[tokio::test]
async fn context_bus_cleared_before_new_mission() {
    let provider = MockProvider;

    let mut plan = Plan::new("Stale context test");
    plan.add_task("Some task", vec![], AgentRole::Analyst);

    // Poison the context before the run to simulate stale data.
    plan.context
        .data
        .insert("stale_key".to_string(), "stale_value".to_string());

    let _ = run_mission(&mut plan, &provider, 3, None).await;

    assert!(
        !plan.context.data.contains_key("stale_key"),
        "Stale context key survived — run_mission did not call context.clear()"
    );
}

// ── Wasm executor tests ───────────────────────────────────────────────────────

/// Compile-time path to `professionals/calculator.wasm` relative to the
/// axion-core crate root.  The binary must be built separately:
///
///   cd axion-professionals/calculator
///   cargo build --target wasm32-wasip1 --release
///   cp target/wasm32-wasip1/release/axion-calculator.wasm \
///      ../../axion-core/professionals/calculator.wasm
const CALC_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/professionals/calculator.wasm"
);

const MANIFESTS_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/professionals/manifests"
);

/// Direct WasmExecutor smoke-test: add two numbers, expect "Result: 42".
/// Skips gracefully when the binary has not been built yet.
#[test]
fn wasm_executor_add_returns_correct_result() {
    let wasm_path = std::path::Path::new(CALC_WASM);
    if !wasm_path.exists() {
        eprintln!(
            "SKIP wasm_executor_add_returns_correct_result — {} not found.\n\
             Build it with:\n  \
             cd axion-professionals/calculator && \
             cargo build --target wasm32-wasip1 --release\n  \
             cp target/wasm32-wasip1/release/axion-calculator.wasm \
             ../../axion-core/professionals/calculator.wasm",
            CALC_WASM
        );
        return;
    }

    let result = WasmExecutor::run(wasm_path, r#"{"operation":"add","values":[10,32]}"#);

    assert!(result.is_ok(), "WasmExecutor::run failed: {:?}", result.err());
    assert_eq!(result.unwrap().trim(), "Result: 42");
}

/// Verify that `execute_tool` routes through the Wasm sandbox when
/// `professionals/calculator.wasm` exists and the registry is initialised.
///
/// This is the end-to-end dispatch test: registry → Wasm path check →
/// WasmExecutor → result.
#[tokio::test]
async fn execute_tool_dispatches_calculator_via_wasm() {
    let manifests = std::path::Path::new(MANIFESTS_DIR);
    let wasm_path = std::path::Path::new(CALC_WASM);

    if !manifests.exists() || !wasm_path.exists() {
        eprintln!(
            "SKIP execute_tool_dispatches_calculator_via_wasm — \
             manifests dir or calculator.wasm not found."
        );
        return;
    }

    // `init` is a no-op if another test already initialised the global.
    axion_core::registry::Registry::init(manifests);

    let result = axion_core::tools::execute_tool(
        "calculator",
        r#"{"operation":"multiply","values":[6,7]}"#,
    )
    .await;

    assert!(result.is_ok(), "Wasm-dispatched calculator failed: {:?}", result.err());
    assert_eq!(
        result.unwrap().trim(),
        "Result: 42",
        "Wasm calculator returned unexpected output"
    );
}

/// ── Existing ContextBus tests ─────────────────────────────────────────────────

/// The ContextBus should contain the result of the task after a successful run,
/// keyed by the task's slug.
#[tokio::test]
async fn context_bus_populated_after_successful_run() {
    let provider = MockProvider;

    let mut plan = Plan::new("Context population test");
    plan.add_task("Find flights to Berlin", vec![], AgentRole::WebSearcher);

    let result = run_mission(&mut plan, &provider, 3, None).await;
    assert!(result.is_ok(), "run_mission should succeed with the mock provider");

    assert!(
        !plan.context.data.is_empty(),
        "ContextBus should be populated with task results after a successful run"
    );

    let value = plan.context.data.values().next().unwrap();
    assert_eq!(value, "mock result", "Context value should be the mock provider's response");
}
