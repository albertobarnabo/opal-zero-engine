use async_trait::async_trait;
use opalzero_core::engine::{AiProvider, ToolResponse};
use opalzero_core::executor::wasm::WasmExecutor;
use opalzero_core::governor::BuiltinGovernor;
use opalzero_core::planner::Plan;
use opalzero_core::protocol::{AgentRole, MissionUpdate, Tool, WasmPreopen};
use opalzero_core::{resume_mission, run_mission};

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
            // Governor quality-check call — always approve.
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
        let governor = BuiltinGovernor::new();
        let _ = run_mission(&mut plan, &provider, &governor, 3, Some(tx)).await;
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
        let governor = BuiltinGovernor::new();
        let _ = run_mission(&mut plan, &provider, &governor, 3, Some(tx)).await;
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
    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new("Stale context test");
    plan.add_task("Some task", vec![], AgentRole::Analyst);

    // Poison the context before the run to simulate stale data.
    plan.context
        .data
        .insert("stale_key".to_string(), "stale_value".to_string());

    let _ = run_mission(&mut plan, &provider, &governor, 3, None).await;

    assert!(
        !plan.context.data.contains_key("stale_key"),
        "Stale context key survived — run_mission did not call context.clear()"
    );
}

// ── Wasm executor tests ───────────────────────────────────────────────────────

/// Compile-time path to `professionals/calculator.wasm` relative to the
/// opalzero-core crate root.  The binary must be built separately:
///
///   cd opalzero-professionals/calculator
///   cargo build --target wasm32-wasip1 --release
///   cp target/wasm32-wasip1/release/opalzero-calculator.wasm \
///      ../../opalzero-core/professionals/calculator.wasm
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
             cd opalzero-professionals/calculator && \
             cargo build --target wasm32-wasip1 --release\n  \
             cp target/wasm32-wasip1/release/opalzero-calculator.wasm \
             ../../opalzero-core/professionals/calculator.wasm",
            CALC_WASM
        );
        return;
    }

    let result = WasmExecutor::run(wasm_path, r#"{"operation":"add","values":[10,32]}"#, &[]);

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
    opalzero_core::registry::Registry::init(manifests);

    let result = opalzero_core::tools::execute_tool(
        "calculator",
        r#"{"operation":"multiply","values":[6,7]}"#,
        "test",
        &opalzero_core::tools::RequestKeys::default(),
    )
    .await;

    assert!(result.is_ok(), "Wasm-dispatched calculator failed: {:?}", result.err());
    assert_eq!(
        result.unwrap().trim(),
        "Result: 42",
        "Wasm calculator returned unexpected output"
    );
}

// ── Memory tool tests ─────────────────────────────────────────────────────────
//
// All tests that write / read / delete the synthetic snapshot must hold
// MEMORY_LOCK for their entire duration so they don't race each other.
// (Same pattern as EXEC_LOCK in the python tests.)
static MEMORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const MEMORY_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/professionals/memory.wasm"
);

/// Path used by the memory tests for a synthetic mission snapshot.
/// Lives inside the workspace-level `missions/` directory so the WASI preopen
/// at `"missions"` (relative to CWD = workspace root) exposes it correctly.
const TEST_SNAPSHOT_ID: &str = "mission_0000000000_test_memory_tool";

/// Write a minimal but valid MissionSnapshot JSON that the memory WASM can parse.
fn write_test_snapshot() {
    let dir = std::path::Path::new("missions");
    std::fs::create_dir_all(dir).expect("could not create missions/");
    let path = dir.join(format!("{}.json", TEST_SNAPSHOT_ID));
    let content = r#"{
  "id": "mission_0000000000_test_memory_tool",
  "timestamp": 0,
  "intent": "What is the capital of France?",
  "task_count": 1,
  "expanded_task_count": 0,
  "status": "completed",
  "layout_hint": "Itinerary",
  "context": {
    "data": {
      "capital_france": "The capital of France is Paris."
    }
  }
}"#;
    std::fs::write(path, content).expect("could not write test snapshot");
}

/// Remove the synthetic snapshot created by write_test_snapshot().
fn remove_test_snapshot() {
    let path = std::path::Path::new("missions")
        .join(format!("{}.json", TEST_SNAPSHOT_ID));
    let _ = std::fs::remove_file(path);
}

/// Direct WasmExecutor smoke-test for the memory tool:
/// look up a synthetic snapshot by exact ID, verify the summary is correct.
#[test]
fn wasm_memory_recalls_snapshot_by_id() {
    let _guard = MEMORY_LOCK.lock().expect("MEMORY_LOCK poisoned");
    let wasm_path = std::path::Path::new(MEMORY_WASM);
    if !wasm_path.exists() {
        eprintln!(
            "SKIP wasm_memory_recalls_snapshot_by_id — {} not found.\n\
             Build it with:\n  \
             cd opalzero-professionals/memory && \
             cargo build --target wasm32-wasip1 --release\n  \
             cp target/wasm32-wasip1/release/opalzero-memory.wasm \
             ../../opalzero-core/professionals/memory.wasm",
            MEMORY_WASM
        );
        return;
    }

    write_test_snapshot();
    // Preopen `missions/` (relative to CWD = workspace root) as `/missions`
    // inside the Wasm guest — identical to what the host does in production.
    let preopen = WasmPreopen {
        host: "missions".to_string(),
        guest: "/missions".to_string(),
        readonly: true,
    };

    let args = format!(r#"{{"mission_id":"{}"}}"#, TEST_SNAPSHOT_ID);
    let result = WasmExecutor::run(wasm_path, &args, &[preopen]);

    remove_test_snapshot();

    assert!(result.is_ok(), "memory Wasm failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("capital of France"),
        "output should mention the mission intent; got:\n{}",
        output
    );
    assert!(
        output.contains("Paris"),
        "output should include the context value 'Paris'; got:\n{}",
        output
    );
}

/// Fuzzy-query test: search by keyword rather than exact ID.
#[test]
fn wasm_memory_recalls_snapshot_by_query() {
    let _guard = MEMORY_LOCK.lock().expect("MEMORY_LOCK poisoned");
    let wasm_path = std::path::Path::new(MEMORY_WASM);
    if !wasm_path.exists() {
        eprintln!("SKIP wasm_memory_recalls_snapshot_by_query — memory.wasm not found.");
        return;
    }

    write_test_snapshot();
    let preopen = WasmPreopen {
        host: "missions".to_string(),
        guest: "/missions".to_string(),
        readonly: true,
    };

    let result = WasmExecutor::run(
        wasm_path,
        r#"{"query":"capital France"}"#,
        &[preopen],
    );

    remove_test_snapshot();

    assert!(result.is_ok(), "memory Wasm failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Paris"),
        "fuzzy query should surface the Paris snapshot; got:\n{}",
        output
    );
}

/// End-to-end dispatch test: execute_tool("memory", …) routes through the
/// Wasm sandbox, which reads the preopened missions/ directory.
#[tokio::test]
async fn execute_tool_dispatches_memory_via_wasm() {
    let _guard = MEMORY_LOCK.lock().expect("MEMORY_LOCK poisoned");
    let wasm_path = std::path::Path::new(MEMORY_WASM);
    if !wasm_path.exists() {
        eprintln!("SKIP execute_tool_dispatches_memory_via_wasm — memory.wasm not found.");
        return;
    }

    let manifests = std::path::Path::new(MANIFESTS_DIR);
    if !manifests.exists() {
        eprintln!("SKIP execute_tool_dispatches_memory_via_wasm — manifests dir not found.");
        return;
    }

    write_test_snapshot();
    // Registry init is a no-op if already set by another test in this process.
    opalzero_core::registry::Registry::init(manifests);

    let args = format!(r#"{{"mission_id":"{}"}}"#, TEST_SNAPSHOT_ID);
    let result = opalzero_core::tools::execute_tool("memory", &args, "test", &opalzero_core::tools::RequestKeys::default()).await;

    remove_test_snapshot();

    assert!(result.is_ok(), "execute_tool(memory) failed: {:?}", result.err());
    let output = result.unwrap();
    assert!(
        output.contains("Paris"),
        "dispatch through execute_tool should return Paris data; got:\n{}",
        output
    );
}

/// Mission-level test: a MockProvider that returns a `memory` tool-call when
/// given the right prompt.  Verifies that the full mission pipeline (plan →
/// dispatcher → execute_tool → Wasm memory → result) produces output
/// containing the recalled mission data.
#[tokio::test]
async fn mission_with_memory_tool_retrieves_past_result() {
    let _guard = MEMORY_LOCK.lock().expect("MEMORY_LOCK poisoned");
    let wasm_path = std::path::Path::new(MEMORY_WASM);
    let manifests = std::path::Path::new(MANIFESTS_DIR);
    if !wasm_path.exists() || !manifests.exists() {
        eprintln!("SKIP mission_with_memory_tool_retrieves_past_result — binaries not present.");
        return;
    }

    write_test_snapshot();
    opalzero_core::registry::Registry::init(manifests);

    // A provider that, on the first (tool-bearing) call, returns a `memory`
    // tool-call pointing at our test snapshot.  On the follow-up call (after
    // the tool result is submitted) it returns a prose summary.
    struct MemoryMockProvider;

    #[async_trait]
    impl AiProvider for MemoryMockProvider {
        async fn generate_response(
            &self,
            _prompt: &str,
            tools: Option<Vec<Tool>>,
        ) -> Result<ToolResponse, String> {
            if tools.is_none() {
                // Governor quality-check call
                return Ok(ToolResponse::Text(
                    r#"{"verdict":"SUCCESS","reasoning":"ok","new_tasks":[]}"#.to_string(),
                ));
            }
            // First call with tools → invoke the memory tool
            Ok(ToolResponse::ToolCall {
                id: "call_mem_001".to_string(),
                name: "memory".to_string(),
                arguments: format!(r#"{{"mission_id":"{}"}}"#, TEST_SNAPSHOT_ID),
            })
        }

        async fn submit_tool_result(
            &self,
            _prompt: &str,
            _tools: Option<Vec<Tool>>,
            _id: &str,
            _name: &str,
            _args: &str,
            tool_result: &str,
        ) -> Result<ToolResponse, String> {
            // Echo the memory result back as the task answer
            Ok(ToolResponse::Text(format!(
                "Recalled from memory: {}",
                tool_result
            )))
        }
    }

    let provider = MemoryMockProvider;
    let governor = BuiltinGovernor::new();
    let mut plan = Plan::new("What was the result of the test mission?");
    plan.add_task(
        &format!(
            "Recall the past mission '{}' using the memory tool and summarise its findings.",
            TEST_SNAPSHOT_ID
        ),
        vec![],
        AgentRole::Analyst,
    );

    let result = run_mission(&mut plan, &provider, &governor, 2, None).await;
    remove_test_snapshot();

    assert!(result.is_ok(), "mission failed: {:?}", result.err());

    // The task result should contain data from the recalled snapshot.
    let task_result = plan.tasks[0].result.as_deref().unwrap_or("");
    assert!(
        task_result.contains("Paris") || task_result.contains("France"),
        "task result should contain recalled snapshot data; got:\n{}",
        task_result
    );
}

// ── Vision tool tests ─────────────────────────────────────────────────────────

static VISION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const TEST_IMAGE_NAME: &str = "test_chart.png";

/// Write a minimal but valid 1×1 red PNG (67 bytes) to `uploads/`.
fn write_test_image() {
    let dir = std::path::Path::new("uploads");
    std::fs::create_dir_all(dir).expect("could not create uploads/");
    // Minimal 1×1 red PNG constructed from raw bytes.
    #[rustfmt::skip]
    let png: &[u8] = &[
        0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A, // PNG signature
        0x00,0x00,0x00,0x0D,0x49,0x48,0x44,0x52, // IHDR length + type
        0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x01, // 1×1 px
        0x08,0x02,0x00,0x00,0x00,0x90,0x77,0x53, // 8-bit RGB, CRC start
        0xDE,0x00,0x00,0x00,0x0C,0x49,0x44,0x41, // IDAT length + type
        0x54,0x08,0xD7,0x63,0xF8,0xCF,0xC0,0x00, // compressed red pixel
        0x00,0x00,0x02,0x00,0x01,0xE2,0x21,0xBC, // CRC
        0x33,0x00,0x00,0x00,0x00,0x49,0x45,0x4E, // IEND length + type
        0x44,0xAE,0x42,0x60,0x82,                 // IEND CRC
    ];
    std::fs::write(dir.join(TEST_IMAGE_NAME), png).expect("could not write test image");
}

fn remove_test_image() {
    let _ = std::fs::remove_file(std::path::Path::new("uploads").join(TEST_IMAGE_NAME));
}

/// Full mission pipeline test for the vision tool.
///
/// The `VisionMockProvider`:
///   1. Returns a `vision` tool-call on the first (tool-bearing) request.
///   2. Synthesises a canned analysis when `submit_tool_result` is called with
///      the tool output (file metadata from the native fallback).
///   3. Returns a Governor SUCCESS verdict on any no-tool request.
///
/// No `OPENAI_API_KEY` is required — the native fallback returns file metadata.
#[tokio::test]
async fn mission_with_image_input() {
    let _guard = VISION_LOCK.lock().expect("VISION_LOCK poisoned");
    write_test_image();

    struct VisionMockProvider;

    #[async_trait]
    impl AiProvider for VisionMockProvider {
        async fn generate_response(
            &self,
            _prompt: &str,
            tools: Option<Vec<Tool>>,
        ) -> Result<ToolResponse, String> {
            if tools.is_none() {
                // Governor quality-check call — approve immediately.
                return Ok(ToolResponse::Text(
                    r#"{"verdict":"SUCCESS","reasoning":"vision analysis complete",
                       "new_tasks":[],"issues":"","refinement_instructions":""}"#
                        .to_string(),
                ));
            }
            // First call with tools → invoke the vision tool.
            Ok(ToolResponse::ToolCall {
                id: "call_vis_001".to_string(),
                name: "vision".to_string(),
                arguments: format!(
                    r#"{{"file_path":"{}","prompt":"Describe the chart data"}}"#,
                    TEST_IMAGE_NAME
                ),
            })
        }

        async fn submit_tool_result(
            &self,
            _prompt: &str,
            _tools: Option<Vec<Tool>>,
            _id: &str,
            _name: &str,
            _args: &str,
            tool_result: &str,
        ) -> Result<ToolResponse, String> {
            // Synthesize a final answer from the vision tool output.
            Ok(ToolResponse::Text(format!(
                "Image analysis complete. Vision tool returned: {}",
                tool_result
            )))
        }
    }

    let provider = VisionMockProvider;
    let governor = BuiltinGovernor::new();
    let mut plan = Plan::new("Analyze the uploaded chart");
    plan.add_task(
        &format!(
            "Use the vision tool to analyze '{}' and describe the data it contains.",
            TEST_IMAGE_NAME
        ),
        vec![],
        AgentRole::Analyst,
    );

    let result = run_mission(&mut plan, &provider, &governor, 2, None).await;
    remove_test_image();

    assert!(result.is_ok(), "mission failed: {:?}", result.err());

    let task_result = plan.tasks[0].result.as_deref().unwrap_or("");
    assert!(
        task_result.contains("test_chart")
            || task_result.contains("image")
            || task_result.contains("png")
            || task_result.contains("Image"),
        "task result should reference the image analysis; got:\n{}",
        task_result
    );
}

/// ── Existing ContextBus tests ─────────────────────────────────────────────────

/// The Governor should inject a refinement task when the Auditor returns REVISE,
/// and the plan should contain the original task plus one refinement task.
///
/// Provider behaviour (tracked by AtomicU32 call counter):
///   n=0  tools=Some  → flat unstructured text (task 0 result)
///   n=1  tools=None  → REVISE verdict (first Governor round)
///   n=2  tools=Some  → flat text again (refinement task result)
///   n=3  tools=None  → SUCCESS verdict (second Governor round, cap hit = approve)
#[tokio::test]
async fn mission_with_quality_correction() {
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    struct RefiningProvider {
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl AiProvider for RefiningProvider {
        async fn generate_response(
            &self,
            _prompt: &str,
            tools: Option<Vec<Tool>>,
        ) -> Result<ToolResponse, String> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if tools.is_some() {
                // Task execution — return short flat text (stays below 80-byte UI threshold).
                Ok(ToolResponse::Text("flat result".to_string()))
            } else if n == 1 {
                // First Governor call → REVISE: inject a synthesis task.
                Ok(ToolResponse::Text(
                    r#"{"verdict":"REVISE","reasoning":"result lacks synthesis",
                       "issues":"no total was calculated",
                       "refinement_instructions":"calculate total and present clearly",
                       "suggested_tasks":[{"description":"Synthesise findings and calculate total trip cost","role":"Analyst"}]}"#
                        .to_string(),
                ))
            } else {
                // All subsequent Governor calls → approve.
                Ok(ToolResponse::Text(
                    r#"{"verdict":"SUCCESS","reasoning":"quality sufficient",
                       "new_tasks":[],"issues":"","refinement_instructions":""}"#
                        .to_string(),
                ))
            }
        }

        async fn submit_tool_result(
            &self,
            _prompt: &str,
            _tools: Option<Vec<Tool>>,
            _id: &str,
            _name: &str,
            _args: &str,
            _result: &str,
        ) -> Result<ToolResponse, String> {
            Ok(ToolResponse::Text("refined result".to_string()))
        }
    }

    let call_count = Arc::new(AtomicU32::new(0));
    let provider = RefiningProvider {
        call_count: Arc::clone(&call_count),
    };
    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new("Calculate total trip cost");
    plan.add_task(
        "Research flight and hotel prices",
        vec![],
        AgentRole::WebSearcher,
    );

    let result = run_mission(&mut plan, &provider, &governor, 3, None).await;

    assert!(result.is_ok(), "mission should complete successfully: {:?}", result.err());
    assert_eq!(
        plan.tasks.len(),
        2,
        "Governor should have injected exactly one refinement task; found {} task(s)",
        plan.tasks.len()
    );

    // The refinement task description must mention the identified issues.
    let refinement_intent = &plan.tasks[1].intent;
    let intent_lower = refinement_intent.to_lowercase();
    assert!(
        intent_lower.contains("synthes") || intent_lower.contains("fix")
            || intent_lower.contains("calculat") || intent_lower.contains("total"),
        "refinement task intent should reference the quality issues; got: {}",
        refinement_intent
    );
}

// ── Human-in-the-loop (HITL) tests ───────────────────────────────────────────

/// Full HITL lifecycle:
///
/// 1. The first `run_mission` call runs a single Analyst task that calls the
///    `feedback` tool — the mission pauses with a [`HandshakeRequest`].
/// 2. The task result carries the `__AWAITING_FEEDBACK__: ` marker.
/// 3. `resume_mission` is called with a user reply.
/// 4. A feedback-driven refinement task is appended; the mission completes.
/// 5. The plan ends up with at least 2 tasks and `user_feedback` in the context.
///
/// Provider call sequence (tracked by `AtomicU32`):
///   n=0  tools=Some  → `feedback` ToolCall (first task)
///   n=1  tools=Some  → plain text result  (refinement task after resume)
///   n=2  tools=None  → Governor SUCCESS   (quality check after resume)
#[tokio::test]
async fn mission_requires_human_approval() {
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    };

    struct HitlProvider {
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl AiProvider for HitlProvider {
        async fn generate_response(
            &self,
            _prompt: &str,
            tools: Option<Vec<Tool>>,
        ) -> Result<ToolResponse, String> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            match (tools.is_some(), n) {
                // First task execution — call the feedback tool to pause.
                (true, 0) => Ok(ToolResponse::ToolCall {
                    id: "call_fb_001".to_string(),
                    name: "feedback".to_string(),
                    arguments: r#"{"question":"Initial analysis is complete. Does this meet your requirements?","context":"Research phase done."}"#
                        .to_string(),
                }),
                // Refinement task after resume — return a plain text answer.
                (true, _) => Ok(ToolResponse::Text(
                    "User feedback applied. Results updated per instructions.".to_string(),
                )),
                // Governor quality check — approve so the mission completes.
                (false, _) => Ok(ToolResponse::Text(
                    r#"{"verdict":"SUCCESS","reasoning":"feedback integrated","new_tasks":[],
                       "issues":"","refinement_instructions":""}"#
                        .to_string(),
                )),
            }
        }

        async fn submit_tool_result(
            &self,
            _prompt: &str,
            _tools: Option<Vec<Tool>>,
            _id: &str,
            _name: &str,
            _args: &str,
            _result: &str,
        ) -> Result<ToolResponse, String> {
            // feedback is a terminal tool — submit_tool_result is never called for it.
            Ok(ToolResponse::Text("mock submit result".to_string()))
        }
    }

    let call_count = Arc::new(AtomicU32::new(0));
    let provider = HitlProvider {
        call_count: Arc::clone(&call_count),
    };
    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new("Analyse quarterly sales data");
    plan.add_task(
        "Perform the initial analysis of the sales figures",
        vec![],
        AgentRole::Analyst,
    );

    // ── First run: should pause awaiting feedback ─────────────────────────────
    let outcome = run_mission(&mut plan, &provider, &governor, 3, None).await;

    assert!(outcome.is_ok(), "run_mission returned Err: {:?}", outcome.err());
    let handshake = outcome.unwrap();
    assert!(
        handshake.is_some(),
        "Expected HandshakeRequest — mission should have paused at the feedback tool"
    );

    let hs = handshake.unwrap();
    assert!(
        hs.question.contains("Initial analysis") || hs.question.contains("requirements"),
        "HandshakeRequest.question should match the feedback tool argument; got: '{}'",
        hs.question
    );

    // Task result must carry the awaiting-feedback prefix so validate_mission
    // can detect it.  (Before resume strips it.)
    let task_result = plan.tasks[0].result.as_deref().unwrap_or("");
    assert!(
        task_result.starts_with("__AWAITING_FEEDBACK__"),
        "Task result should start with the awaiting-feedback marker; got: '{}'",
        task_result
    );

    // ── Resume with user feedback ─────────────────────────────────────────────
    let user_reply = "Looks good — please add a bullet-point summary of the top 3 findings.";

    let resumed = resume_mission(&mut plan, user_reply, &provider, &governor, 3, None).await;

    assert!(
        resumed.is_ok(),
        "resume_mission returned Err: {:?}",
        resumed.err()
    );
    assert!(
        resumed.unwrap().is_none(),
        "Resumed mission should complete fully — not pause again"
    );

    // ── Post-resume assertions ────────────────────────────────────────────────

    // Plan must have at least 2 tasks: the original + the feedback refinement.
    // The Governor may add further expansion tasks (e.g. UI Builder) — that is
    // expected and fine; the key requirement is that task[1] IS the feedback task.
    assert!(
        plan.tasks.len() >= 2,
        "Plan should contain original task + at least 1 feedback refinement task; got {} task(s)",
        plan.tasks.len()
    );

    // task[1] must be the feedback-driven refinement task injected by resume_mission.
    let refinement_intent = &plan.tasks[1].intent;
    assert!(
        refinement_intent.contains("feedback") || refinement_intent.contains("User"),
        "plan.tasks[1] should be the feedback refinement task; got: '{}'",
        refinement_intent
    );

    // user_feedback must be persisted in the ContextBus.
    assert!(
        plan.context.data.contains_key("user_feedback"),
        "ContextBus should contain 'user_feedback' after resume"
    );
    assert_eq!(
        plan.context.data["user_feedback"],
        user_reply,
        "ContextBus user_feedback value should match the reply passed to resume_mission"
    );

    // The original task result should no longer carry the raw marker — resume
    // strips it so subsequent validate_mission calls don't re-trigger HITL.
    let cleaned_result = plan.tasks[0].result.as_deref().unwrap_or("");
    assert!(
        !cleaned_result.starts_with("__AWAITING_FEEDBACK__"),
        "After resume the marker should have been stripped; got: '{}'",
        cleaned_result
    );
}

/// The ContextBus should contain the result of the task after a successful run,
/// keyed by the task's slug.
#[tokio::test]
async fn context_bus_populated_after_successful_run() {
    let provider = MockProvider;
    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new("Context population test");
    plan.add_task("Find flights to Berlin", vec![], AgentRole::WebSearcher);

    let result = run_mission(&mut plan, &provider, &governor, 3, None).await;
    assert!(result.is_ok(), "run_mission should succeed with the mock provider");

    assert!(
        !plan.context.data.is_empty(),
        "ContextBus should be populated with task results after a successful run"
    );

    let value = plan.context.data.values().next().unwrap();
    assert_eq!(value, "mock result", "Context value should be the mock provider's response");
}
