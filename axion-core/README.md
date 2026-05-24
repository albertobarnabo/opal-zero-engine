# axion-engine

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)
![npm](https://img.shields.io/npm/v/axion-sdk.svg)

**A Rust-native multi-agent intelligence kernel. Give it an intent — it plans, executes, validates, and streams structured results back to you.**

Most agent frameworks hand you a pile of primitives and wish you luck. axion-engine is an opinionated kernel: it takes a plain-English intent, breaks it into a dependency-ordered task graph, dispatches each task to a specialist agent (Analyst, WebSearcher), runs the combined output through a five-criterion quality rubric, and either approves it or sends agents back to fix specific gaps. No manual orchestration. No prompt babysitting.

---

## Quickstart

The fastest path is the HTTP server + React SDK. No Rust required.

**1 — Run the server**

```bash
git clone https://github.com/albertobarnabo/axion-engine
cd axion-engine
export OPENAI_API_KEY=sk-...
PORT=3491 cargo run --bin axion-server
```

**2 — Install the SDK**

```bash
npm install axion-sdk
```

**3 — Use it in React**

```tsx
import { useAxion } from "axion-sdk";

export function Brief() {
  const { data, agents, status } = useAxion(
    "Interview brief for Software Engineer at Stripe",
    { config: { serverUrl: "http://localhost:3491" } }
  );

  return <pre>{JSON.stringify(data, null, 2)}</pre>;
}
```

That's it. The kernel plans, searches, analyses, and validates. You get structured JSON.

---

## Providers

axion-server supports multiple AI backends, selected via the `AXION_PROVIDER` environment variable.

### OpenAI (default)

```bash
AXION_PROVIDER=openai AXION_MODEL=gpt-4o-mini OPENAI_API_KEY=sk-... cargo run --bin axion-server
```

### Ollama (local, free)

Run any model locally with no API key required. Axion relies on tool calling, so you need a model that supports it.

```bash
ollama pull llama3.1:8b
AXION_PROVIDER=ollama AXION_MODEL=llama3.1:8b cargo run --bin axion-server
```

**Recommended models** (all support tool calling):

| Model | Size | Notes |
|---|---|---|
| `llama3.1:8b` | 4.7 GB | Best balance of speed and quality — default |
| `mistral-nemo` | 7.1 GB | Strong reasoning, great for Analyst tasks |
| `qwen2.5:7b` | 4.7 GB | Fast, good tool-call reliability |

> **⚠️ Not recommended:** `llama3.2:3b`, `phi3:mini`, `tinyllama` — these models do not support tool calling and missions will fail. The server will print a warning if you try to use one.

### Anthropic Claude

```bash
AXION_PROVIDER=claude AXION_MODEL=claude-sonnet-4-5 ANTHROPIC_API_KEY=sk-ant-... cargo run --bin axion-server
```

Defaults to `claude-sonnet-4-5`. Haiku is automatically used for cheaper sub-tasks (web search, code execution) while the selected model handles planning and analysis.

### Any OpenAI-compatible endpoint (Groq, Together, Mistral…)

```bash
AXION_PROVIDER=compatible \
  AXION_BASE_URL=https://api.groq.com/openai/v1 \
  AXION_MODEL=llama-3.3-70b-versatile \
  AXION_API_KEY=gsk_... \
  cargo run --bin axion-server
```

### Provider env vars

| Env var | Default | Description |
|---|---|---|
| `AXION_PROVIDER` | `openai` | Backend: `openai`, `claude`, `ollama`, or `compatible` |
| `AXION_MODEL` | `gpt-4o-mini` / `llama3.1:8b` | Model name (default varies by provider) |
| `AXION_BASE_URL` | — | Required when `AXION_PROVIDER=compatible` |
| `AXION_API_KEY` | — | API key for compatible endpoints |

---

## Output schema contract

Pass a typed schema and the Analyst is contractually bound to produce exactly those keys — no hallucinated fields, no missing keys, no extra output to wrangle downstream.

```tsx
const SCHEMA = {
  current_price_usd:  "number",
  market_cap_usd:     "number",
  top_competitors:    "array",
  analyst_consensus:  "string",
  recent_news:        "array",
};

const { data } = useAxion("Financial brief for Apple", {
  schema: SCHEMA,
  config: { serverUrl: "http://localhost:3491" },
});

// data is guaranteed to have exactly these keys, nothing else
```

The Governor enforces the contract. If the Analyst drifts, it gets sent back. The result you receive always matches the shape you declared.

---

## How it works

```
User intent
    │
    ▼
 Planner ──── builds a dependency-ordered task graph
    │
    ├──► WebSearcher   ──┐
    ├──► WebSearcher   ──┼──► ContextBus (shared memory)
    └──► Analyst       ──┘
                          │
                          ▼
                      Governor
                  (5-criterion rubric)
                          │
              ┌───────────┴───────────┐
              ▼                       ▼
         MissionComplete          REVISE → inject fix tasks → loop
```

Results stream back over SSE as each task completes. The React SDK wraps the stream into a `useAxion()` hook with live agent status via `AgentStrip`.

---

## Benchmarks

Independent evaluation against 7 agent frameworks (LangChain, LangGraph, AutoGen, CrewAI, LlamaIndex, smolagents, PydanticAI) across 5 real-world task types and 10 dimensions:

| Dimension | Axion | Next best |
|---|---|---|
| Task decomposition | 5/5 | LangGraph 4/5 |
| Schema compliance | 5/5 | PydanticAI 5/5 |
| Tool breadth | 5/5 | LangChain 5/5 |
| Quality gate (built-in) | 5/5 | none others 5/5 |
| Multi-provider support | 5/5 | AutoGen 4/5 |
| **Total** | **44 / 50** | **LangGraph 35/50** |

Full methodology, scoring rubrics, per-framework profiles, and performance numbers: **[BENCHMARK.md](./BENCHMARK.md)**

---

## What makes it different

- **Schema contract** — declare the exact output shape you need; the kernel enforces it end-to-end. No post-processing, no field mapping.
- **Structured quality gates** — the Governor scores output against five explicit rubrics (intent coverage, data density, structural validity, claim specificity, synthesis completeness) and rejects vague results.
- **Task dependency graph** — tasks declare `depends_on` slugs; the dispatcher withholds a task until its dependencies complete and cascade-fails anything blocked by a failed upstream.
- **Self-hosted** — one binary, your own API key, your own infra. Your data never touches anyone else's infrastructure.
- **Engine retry with backoff** — every LLM call retries up to 3 times with exponential backoff. Permanent failures short-circuit immediately.
- **Configurable wall-clock timeout** — missions that hang emit `MissionFailed` after a deadline you control.

---

## Configuration

| Env var | Default | Description |
|---|---|---|
| `OPENAI_API_KEY` | required | LLM provider key |
| `ALPHA_VANTAGE_API_KEY` | optional | Enables financial data tools |
| `ANTHROPIC_API_KEY` | optional | Required when `AXION_PROVIDER=claude` |
| `AXION_SMTP_HOST` | optional | SMTP host — enables the `send_email` tool |
| `AXION_SMTP_USER` | optional | SMTP username |
| `AXION_SMTP_PASS` | optional | SMTP password / app password |
| `AXION_SMTP_FROM` | optional | Sender address |
| `PORT` | 8080 | axion-server listen port |
| `AXION_MAX_TOKENS` | 4096 | Max tokens per LLM response |
| `AXION_TEMPERATURE` | 0.1 | Sampling temperature |
| `AXION_MISSION_TIMEOUT_SECS` | 300 | Wall-clock deadline per mission |
| `AXION_MAX_LLM_RETRIES` | 3 | Retry attempts on transient LLM errors |

---

## Embed directly in Rust

If you want the kernel without the HTTP layer:

```rust
use axion_core::prelude::*;
use axion_core::engine::SimpleProvider;
use axion_core::governor::BuiltinGovernor;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let provider = SimpleProvider::openai("gpt-4o-mini").unwrap();
    let governor = BuiltinGovernor::new();
    let (tx, mut rx) = mpsc::channel::<MissionUpdate>(64);

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
                println!("🎯 Done — missions/{mission_id}.json"),
            MissionUpdate::MissionFailed { error } =>
                eprintln!("❌ {error}"),
            _ => {}
        }
    }
}
```

```toml
[dependencies]
axion-core = { git = "https://github.com/albertobarnabo/axion-engine" }
tokio      = { version = "1", features = ["full"] }
dotenvy    = "0.15"
```

---

## Related

- [axion-sdk](https://www.npmjs.com/package/axion-sdk) — React SDK (`useAxion`, `AgentStrip`, TypeScript types)
- [axion-professionals](https://github.com/albertobarnabo/axion-professionals) — community WASM tool modules
- [axion-demo](https://github.com/albertobarnabo/axion-demo) — reference apps built on axion-sdk (Lumen, Brief)
- [Docs & landing](https://albertobarnabo.com/axion/)

---

## License

MIT — build whatever you want.

⭐ If axion-engine saves you from writing another orchestration layer, a star helps others find it.
