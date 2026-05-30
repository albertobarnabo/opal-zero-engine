# opalzero-engine

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)
![npm](https://img.shields.io/npm/v/opal-zero.svg)

**Want to use AI in your app but you're not an AI engineer? OpalZero is the one you don't have to hire.** Send it an intent and the exact output schema you want — get back finished, structured data. No prompts, no model wiring, no orchestration code.

You run opalzero-engine as a headless server next to your app — in any language — and hand it the entire AI part. Your app sends an intent (and optionally a schema) and gets structured data back; it never writes a prompt or picks a model.

Most agent frameworks hand you a pile of primitives and wish you luck. opalzero-engine is the opposite — an opinionated kernel: it takes a plain-English intent, breaks it into a dependency-ordered task graph, dispatches each task to a specialist agent (Analyst, WebSearcher), runs the combined output through a five-criterion quality rubric, and either approves it or sends agents back to fix specific gaps. No manual orchestration. No prompt babysitting.

---

## 🔭 OpalGlimpse — the first product built on OpalZero *(coming soon)*

Autonomous monitoring powered by OpalZero: point it at markets, competitors, or any signal — it runs on a schedule and shows you **exactly what changed**, as structured diffs, not noise. *Watch the world change while you sleep.*

It launches as a hosted SaaS, and **we deploy it once there's enough interest.** Want early access?

- 👍 or comment on the **[OpalGlimpse early-access issue →](https://github.com/albertobarnabo/opal-zero-engine/issues/1)**
- or email **albertobarnabo@gmail.com**

Meanwhile, run the OpalZero engine yourself today — bring your own API key.

---

## Quickstart

The fastest path is the HTTP server + React SDK. No Rust required.

**1 — Run the server**

```bash
git clone https://github.com/albertobarnabo/opalzero-engine
cd opalzero-engine
export OPENAI_API_KEY=sk-...
PORT=8000 cargo run --bin opalzero-server
```

**2 — Install the SDK**

```bash
npm install opal-zero
```

**3 — Use it in React**

```tsx
import { OpalZeroClient } from "opal-zero";
import { useOpalZero } from "opal-zero/react";

const client = new OpalZeroClient({ baseUrl: "http://localhost:8000" });

export function Brief() {
  const { run, cards, status } = useOpalZero({ client });

  return (
    <div>
      <button onClick={() => run("Interview brief for a Software Engineer at Stripe")}>
        Run
      </button>
      {status === "complete" &&
        cards.map((c) => <pre key={c.key}>{JSON.stringify(c.props, null, 2)}</pre>)}
    </div>
  );
}
```

That's it. The kernel plans, searches, analyses, and validates. Your app just renders the structured result.

---

## Providers

opalzero-server supports multiple AI backends, selected via the `OPALZERO_PROVIDER` environment variable.

### OpenAI (default)

```bash
OPALZERO_PROVIDER=openai OPALZERO_MODEL=gpt-4o-mini OPENAI_API_KEY=sk-... cargo run --bin opalzero-server
```

### Ollama (local, free)

Run any model locally with no API key required. OpalZero relies on tool calling, so you need a model that supports it.

```bash
ollama pull llama3.1:8b
OPALZERO_PROVIDER=ollama OPALZERO_MODEL=llama3.1:8b cargo run --bin opalzero-server
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
OPALZERO_PROVIDER=claude OPALZERO_MODEL=claude-sonnet-4-5 ANTHROPIC_API_KEY=sk-ant-... cargo run --bin opalzero-server
```

Defaults to `claude-sonnet-4-5`. Haiku is automatically used for cheaper sub-tasks (web search, code execution) while the selected model handles planning and analysis.

### Any OpenAI-compatible endpoint (Groq, Together, Mistral…)

```bash
OPALZERO_PROVIDER=compatible \
  OPALZERO_BASE_URL=https://api.groq.com/openai/v1 \
  OPALZERO_MODEL=llama-3.3-70b-versatile \
  cargo run --bin opalzero-server
```

### Provider env vars

| Env var | Default | Description |
|---|---|---|
| `OPALZERO_PROVIDER` | `openai` | Backend: `openai`, `claude`, `ollama`, or `compatible` |
| `OPALZERO_MODEL` | `gpt-4o-mini` / `llama3.1:8b` | Model name (default varies by provider) |
| `OPALZERO_BASE_URL` | — | Required when `OPALZERO_PROVIDER=compatible` |

---

## Output schema contract

Pass a typed schema and the Analyst is contractually bound to produce exactly those keys — no hallucinated fields, no missing keys, no extra output to wrangle downstream. **This is what makes the kernel general: every problem becomes the same problem — an intent and the shape of its answer.**

```ts
import { OpalZeroClient } from "opal-zero";

const client = new OpalZeroClient({ baseUrl: "http://localhost:8000" });

const SCHEMA = {
  current_price_usd: "number",
  market_cap_usd:    "number",
  top_competitors:   "array",
  analyst_consensus: "string",
  recent_news:       "array",
};

for await (const event of client.execute("Financial brief for Apple", undefined, SCHEMA)) {
  if (event.type === "mission_complete") {
    // event.mission_state.data_payload has exactly these keys, nothing else
    console.log(event.mission_state?.data_payload);
  }
}
```

The Governor enforces the contract. If the Analyst drifts, it gets sent back. The result you receive always matches the shape you declared. Python (`oz.execute(intent, schema=SCHEMA)`) and raw `curl` accept the same `schema` field.

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

Results stream back over SSE as each task completes. The React SDK wraps the stream into a `useOpalZero()` hook with live agent status via `AgentStrip`.

---

## Benchmarks

Independent evaluation against 7 agent frameworks (LangChain, LangGraph, AutoGen, CrewAI, LlamaIndex, smolagents, PydanticAI) across 5 real-world task types and 10 dimensions:

| Dimension | OpalZero | Next best |
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
| `ANTHROPIC_API_KEY` | optional | Required when `OPALZERO_PROVIDER=claude` |
| `OPALZERO_SMTP_HOST` | optional | SMTP host — enables the `send_email` tool |
| `OPALZERO_SMTP_USER` | optional | SMTP username |
| `OPALZERO_SMTP_PASS` | optional | SMTP password / app password |
| `OPALZERO_SMTP_FROM` | optional | Sender address |
| `PORT` | 8000 | opalzero-server listen port |
| `OPALZERO_MAX_TOKENS` | 4096 | Max tokens per LLM response |
| `OPALZERO_TEMPERATURE` | 0.1 | Sampling temperature |
| `OPALZERO_MISSION_TIMEOUT_SECS` | 300 | Wall-clock deadline per mission |
| `OPALZERO_MAX_LLM_RETRIES` | 3 | Retry attempts on transient LLM errors |

---

## Embed directly in Rust

If you want the kernel without the HTTP layer:

```rust
use opalzero_core::prelude::*;
use opalzero_core::engine::SimpleProvider;
use opalzero_core::governor::BuiltinGovernor;
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
opalzero-core = { git = "https://github.com/albertobarnabo/opalzero-engine" }
tokio      = { version = "1", features = ["full"] }
dotenvy    = "0.15"
```

---

## Related

- [opal-zero](https://www.npmjs.com/package/opal-zero) — TypeScript / React SDK (`OpalZeroClient`, `useOpalZero`, full type defs)
- [opalzero-professionals](https://github.com/albertobarnabo/opalzero-professionals) — community WASM tool modules
- [opalzero-demo](https://github.com/albertobarnabo/opalzero-demo) — reference apps built on opalzero-sdk (Lumen, Brief)
- [Docs & landing](https://albertobarnabo.com/opal-zero/)

---

## License

MIT — build whatever you want.

⭐ If opalzero-engine saves you from writing another orchestration layer, a star helps others find it.
