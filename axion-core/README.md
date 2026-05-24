# axion-core

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)
![npm](https://img.shields.io/npm/v/axion-sdk.svg)
![PyPI](https://img.shields.io/pypi/v/axion-sdk.svg)

**A Rust-native multi-agent intelligence kernel. Give it an intent — it plans, executes, validates, and streams structured results back to you.**

Most agent frameworks hand you a pile of primitives and wish you luck. axion-core is an opinionated kernel: it takes a plain-English intent, breaks it into a dependency-ordered task graph, dispatches each task to a specialist agent (Analyst, Coder, WebSearcher), puts the combined output through a five-criterion quality rubric, and either approves it or sends agents back to fix specific gaps. No manual orchestration. No prompt babysitting.

Unlike LangChain, CrewAI, or AutoGen — which give you raw chains and agents — axion-core gives you a production-grade kernel with structured validation baked in. You get deterministic task ordering, cross-mission memory, exponential-backoff retries, and a configurable wall-clock timeout, all out of the box.

---

## How it works

```
User intent
    │
    ▼
 Planner ──── builds a dependency-ordered task graph
    │
    ├──► WebSearcher   ──┐
    ├──► Analyst          ├──► ContextBus (shared memory)
    └──► Coder        ──┘
                          │
                          ▼
                      Governor
                  (5-criterion rubric)
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
         MissionComplete          REVISE → inject fix tasks → loop
```

---

## Quickstart

```rust
use axion_core::prelude::*;
use axion_core::engine::SimpleProvider;
use axion_core::governor::BuiltinGovernor;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let provider  = SimpleProvider::openai("gpt-4o-mini").unwrap();
    let governor  = BuiltinGovernor::new();
    let (tx, mut rx) = mpsc::channel::<MissionUpdate>(64);

    // Build a plan from natural language, then run it
    let mut plan = build_plan_from_intent(
        "Compare the top 3 EVs under $60k on range, price, and charging speed",
        &provider,
    ).await;

    tokio::spawn(async move {
        run_mission(&mut plan, &provider, &governor, 3, Some(tx)).await.ok();
    });

    while let Some(event) = rx.recv().await {
        match event {
            MissionUpdate::TaskCompleted { slug, result, .. } =>
                println!("✅ {slug}: {}", &result[..result.len().min(120)]),
            MissionUpdate::MissionComplete { mission_id, .. } =>
                println!("🎯 Done — snapshot saved as missions/{mission_id}.json"),
            MissionUpdate::MissionFailed { error } =>
                eprintln!("❌ {error}"),
            _ => {}
        }
    }
}
```

Add to `Cargo.toml`:

```toml
[dependencies]
axion-core = { git = "https://github.com/albertobarnabo/axion-engine" }
tokio       = { version = "1", features = ["full"] }
dotenvy     = "0.15"
```

Set `OPENAI_API_KEY` in your environment and run.

---

## What makes it different

- **Structured quality gates** — the Governor scores output against five explicit rubrics (intent coverage, data density, structural validity, claim specificity, synthesis completeness) and rejects vague results. Not vibes-based self-evaluation.
- **Task dependency graph** — tasks declare `depends_on` slugs; the dispatcher withholds a task until its dependencies complete and cascade-fails anything blocked by a failed upstream task.
- **Cross-mission memory** — agents can write named facts to a persistent store (`memory/global.json`) and retrieve them in any future mission. The kernel automatically injects this context at the start of each run.
- **Engine retry with backoff** — every LLM call retries up to 3 times with exponential backoff. Permanent failures (invalid key, context overflow) short-circuit immediately.
- **Configurable wall-clock timeout** — missions that hang emit `MissionFailed` after a deadline you control. Default: 5 minutes.

---

## Configuration

| Env var | Default | Description |
|---|---|---|
| `AXION_PROVIDER` | `openai` | Backend: `openai`, `claude`, `ollama`, or `compatible` |
| `OPENAI_API_KEY` | — | Required when `AXION_PROVIDER=openai` |
| `ANTHROPIC_API_KEY` | — | Required when `AXION_PROVIDER=claude` |
| `AXION_MODEL` | provider default | Model name (e.g. `claude-sonnet-4-5`, `gpt-4o-mini`) |
| `AXION_MAX_TOKENS` | 4096 | Max tokens per LLM response |
| `AXION_TEMPERATURE` | 0.1 | Sampling temperature |
| `AXION_MISSION_TIMEOUT_SECS` | 300 | Wall-clock deadline per mission |
| `AXION_MAX_LLM_RETRIES` | 3 | Retry attempts on transient LLM errors |
| `AXION_RETRY_BASE_DELAY_MS` | 1000 | Base delay for exponential backoff |

---

## Use with the HTTP server

Pair with **axion-server** for a full REST + SSE API — POST an intent, stream `MissionUpdate` events, retrieve snapshots, trigger refinements. Full docs at [albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs).

---

## License

MIT — build whatever you want.

⭐ If axion-core saves you from writing another orchestration layer, a GitHub star helps others find it.
