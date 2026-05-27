<div align="center">
  <img src="https://albertobarnabo.com/opal-zero/opalzero-logo.svg" width="64" alt="OpalZero" />
  <h1>opal-zero</h1>
</div>

TypeScript SDK for [OpalZero](https://github.com/albertobarnabo/opal-zero-engine) — a self-hosted multi-agent intelligence kernel. Give it a plain-English intent; get back structured results streamed in real time.

```bash
npm install opal-zero
```

---

## What is OpalZero?

OpalZero is a self-hosted server that turns an unstructured natural-language task into a dependency-ordered execution plan, dispatches specialist AI agents to fulfill it, validates every result through an autonomous quality gate (the Governor), and streams the entire process back to your application over SSE.

**Architecture at a glance:**

```
Your app  ──POST /api/v1/execute──▶  Planner
                                        │  produces ordered task graph
                                     Dispatcher
                                        │  assigns agents by role
                              ┌─────────┴──────────┐
                           WebSearcher          Analyst / Coder …
                              │                     │
                           Governor  ◀─────────────┘
                              │  validates; may Expand / Retry / Refine
                           ContextBus
                              │  aggregates final state
          ◀─SSE stream──  mission_complete
```

Agents share a live ContextBus. The Governor scores every result across five criteria; if quality is insufficient it can inject additional tasks mid-run (`governor_expand` event), retry, or pause and ask you for clarification (`awaiting_feedback` event).

---

## Installation

```bash
npm install opal-zero      # npm
yarn add opal-zero         # yarn
pnpm add opal-zero         # pnpm
```

React (`useOpalZero`) is a peer dependency — install React separately if you haven't already:

```bash
npm install react react-dom
```

---

## Quickstart — React

```tsx
import { OpalZeroClient } from "opal-zero";
import { useOpalZero } from "opal-zero/react";

const client = new OpalZeroClient({ baseUrl: "http://localhost:8000" });

export function MissionRunner() {
  const { run, status, cards, activeAgent, error, missionId, refine } =
    useOpalZero({ client });

  return (
    <div>
      <button onClick={() => run("Compare the top 3 EVs under $60k")}>Run</button>
      <button
        disabled={!missionId}
        onClick={() => refine(missionId!, "Add charging infrastructure scores")}
      >
        Refine
      </button>

      {activeAgent && (
        <p>
          {activeAgent.role} — {activeAgent.intent}
        </p>
      )}

      {status === "complete" &&
        cards.map((card) => (
          <div key={card.key}>
            <h3>{card.widget}: {card.key}</h3>
            {card.isRefined && <span>Updated</span>}
            <pre>{JSON.stringify(card.props, null, 2)}</pre>
          </div>
        ))}

      {error && <p>Error: {error}</p>}
    </div>
  );
}
```

---

## Quickstart — Vanilla TypeScript / Node

```ts
import { OpalZeroClient } from "opal-zero";

const client = new OpalZeroClient({ baseUrl: "http://localhost:8000" });

for await (const event of client.execute("Summarise the latest Rust release notes")) {
  switch (event.type) {
    case "task_started":
      console.log(`▶ [${event.role}] ${event.intent}`);
      break;
    case "task_completed":
      console.log(`✅ ${event.slug}:`, event.result);
      break;
    case "governor_expand":
      console.log(`🔁 Governor added ${event.new_task_count} tasks`);
      break;
    case "mission_complete":
      console.log("Done:", event.mission_id, event.mission_state);
      break;
    case "mission_failed":
      console.error("Failed:", event.error);
      break;
  }
}
```

---

## Running the server

### Docker (recommended)

```bash
docker run \
  -e OPENAI_API_KEY=sk-... \
  -e TAVILY_API_KEY=tvly-...   `# optional — enables web search` \
  -e OPALZERO_API_KEY=...      `# optional — enables API key auth` \
  -p 8000:8000 \
  ghcr.io/albertobarnabo/opalzero-server:latest
```

### Docker Compose

```yaml
# docker-compose.yml
services:
  opalzero-server:
    image: ghcr.io/albertobarnabo/opalzero-server:latest
    ports:
      - "8000:8000"
    environment:
      - OPENAI_API_KEY=${OPENAI_API_KEY}
      - TAVILY_API_KEY=${TAVILY_API_KEY:-}
      - OPALZERO_API_KEY=${OPALZERO_API_KEY:-}
    volumes:
      - opalzero-missions:/app/missions
      - opalzero-uploads:/app/uploads

volumes:
  opalzero-missions:
  opalzero-uploads:
```

### From source (Rust)

```bash
git clone https://github.com/albertobarnabo/opal-zero-engine
cd opal-zero-engine
export OPENAI_API_KEY=sk-...
PORT=8000 cargo run --release -p opalzero-server
```

### Environment variables

| Variable | Required | Description |
|---|---|---|
| `OPENAI_API_KEY` | Yes | Primary LLM provider key |
| `TAVILY_API_KEY` | No | Enables the `web_search` tool for real-time retrieval |
| `ALPHA_VANTAGE_API_KEY` | No | Enables financial data tools (stock prices, income statements, news sentiment) |
| `OPALZERO_API_KEY` | No | When set, all API requests must include `X-OpalZero-Key` matching this value |
| `PORT` | No | Listening port (default `8000`) |

---

## API Reference

### `OpalZeroClient`

```ts
import { OpalZeroClient } from "opal-zero";

const client = new OpalZeroClient({
  baseUrl:          "http://localhost:8000", // required
  apiKey?:          string,                 // X-OpalZero-Key header
  openAiKey?:       string,                 // per-request OpenAI key override
  tavilyKey?:       string,                 // per-request Tavily key override
  alphaVantageKey?: string,                 // per-request Alpha Vantage key override
});
```

Per-request key overrides are useful when you want different callers to use their own API keys without the server needing a shared key configured.

---

#### `client.execute(intent, model?)`

Starts a new mission. Returns an `AsyncGenerator<MissionEvent>` that yields SSE events until the mission completes or fails.

```ts
for await (const event of client.execute("Plan a 3-day trip to Rome")) {
  // handle events (see SSE Events below)
}
```

The optional `model` parameter is passed directly to the server and may be used to select a specific LLM (e.g. `"gpt-4o"`, `"claude-opus-4-7"`).

---

#### `client.missions.list()`

Returns `MissionSummary[]` — all past missions with their id, intent, status, and creation timestamp.

```ts
const missions = await client.missions.list();
// [{ id, intent, status, created_at }, ...]
```

---

#### `client.missions.get(id)`

Returns a full `MissionSnapshot` including the complete task plan, every task's result, and the final `MissionState`.

```ts
const snapshot = await client.missions.get("mission-uuid");
// { mission_id, intent, plan, mission_state, status, created_at }
```

---

#### `client.missions.refine(id, intent, model?)`

Streams a refinement on an existing mission. Agents pick up the context from the prior run; new results are merged into the existing state. Returns `AsyncGenerator<MissionEvent>`.

```ts
for await (const event of client.missions.refine(id, "Add a cost breakdown")) {
  // same event types as execute()
}
```

---

#### `client.missions.export(id, format)`

Downloads the mission result. Returns a `Blob` ready to save or display.

```ts
const blob = await client.missions.export(id, "md");   // "md" | "csv" | "html"
const url = URL.createObjectURL(blob);
```

---

#### `client.missions.delete(id)`

Deletes a mission and its persisted data from the server.

```ts
await client.missions.delete(id);
```

---

#### `client.upload(file)`

Uploads a `File` object (CSV, JSON, image — max 10 MB) to make it available as context for agents during a subsequent `execute()` call.

```ts
const result = await client.upload(file);
// { filename: "abc123.csv", file_type: "data", original_name: "sales.csv" }
```

---

#### `client.configStatus()`

Returns which API keys are configured on the server. Useful for showing/hiding features in your UI.

```ts
const status = await client.configStatus();
// { openai: true, tavily: false, alpha_vantage: false }
```

---

### SSE Events

Every event has a `type` discriminant. Use a `switch` on `event.type` to handle them:

| `event.type` | Fields | When |
|---|---|---|
| `task_started` | `slug`, `role`, `intent` | An agent begins working on a task |
| `task_completed` | `slug`, `role`, `result` | An agent finishes; `result` is the agent's raw output string |
| `task_failed` | `slug`, `role` | An agent failed; the mission may still continue if other tasks can proceed |
| `governor_expand` | `new_task_count`, `descriptions[]` | The Governor rejected output and injected additional tasks to fill gaps |
| `mission_complete` | `mission_id`, `intent`, `task_count`, `expanded_task_count`, `layout_hint`, `mission_state?` | All tasks done; `mission_state` has the final structured result |
| `mission_failed` | `error` | Unrecoverable failure; `error` contains the reason |
| `mission_paused` | `question`, `mission_id` | The `feedback` tool paused execution awaiting human input |
| `awaiting_feedback` | `slug`, `question` | A specific task is paused for clarification |

**TypeScript event types:**

```ts
import type {
  MissionEvent,
  TaskStartedEvent,
  TaskCompletedEvent,
  TaskFailedEvent,
  GovernorExpandEvent,
  MissionCompleteEvent,
  MissionFailedEvent,
  MissionPausedEvent,
  AwaitingFeedbackEvent,
} from "opal-zero";
```

---

### `useOpalZero(options)` · `opal-zero/react`

```ts
import { useOpalZero } from "opal-zero/react";

const {
  run,          // (intent: string, model?: string) => Promise<void>
  refine,       // (missionId: string, intent: string, model?: string) => Promise<void>
  status,       // "idle" | "running" | "complete" | "failed"
  cards,        // BentoCard[] — ready-to-render result cards
  activeAgent,  // { role: string; intent: string } | null
  error,        // string | null
  missionId,    // string | null — pass to refine()
  missionState, // MissionState | null — raw payload for custom renderers
  reset,        // () => void — reset to idle
} = useOpalZero({
  client,       // OpalZeroClient instance
  model?,       // default model string
  onEvent?,     // (event: MissionEvent) => void — tap every event for side effects
});
```

**`run(intent, model?)`** — clears all previous state and starts a new mission.

**`refine(missionId, intent, model?)`** — leaves the existing card grid in place and merges new results. Cards added or updated by the refinement are marked `isRefined: true`. Does nothing if `status === "running"`.

**`reset()`** — resets to `idle`. Does not abort an in-flight stream — call it after the stream ends or as a UI affordance.

**`activeAgent`** — non-null while a `task_started` event has been received and no `task_completed` / `task_failed` has followed yet. Use this to show a live "Agent running…" indicator.

**`onEvent`** — called for every SSE event before the hook updates its own state. Use for side effects (trace logs, toast banners) that live outside the hook.

---

### `parseBentoCards(state, options?)`

Converts a raw `MissionState` into an ordered `BentoCard[]`. Used internally by `useOpalZero` — export it when building a custom renderer.

```ts
import { parseBentoCards } from "opal-zero";

const cards = parseBentoCards(missionState);
// or with refinement tracking:
const cards = parseBentoCards(missionState, { refinedKeys: new Set(["ev_range"]) });
```

**How it works:**

The server's Analyst agent produces a `MissionState` with two parts:
- `data_payload` — a key/value map of structured results (strings, numbers, objects, arrays)
- `suggested_widgets` — an ordered list of `"WidgetType:key"` strings that map payload entries to UI components

`parseBentoCards` joins these to produce `BentoCard[]`. When `suggested_widgets` is absent, widgets are inferred from the shape of each payload value (number → `MetricCard`, URL string → `ImageCard`, array of objects → `ComparisonTable`).

**Widget types:**

| Widget | When used |
|---|---|
| `MetricCard` | Scalar values — numbers, short strings |
| `ChartCard` | Arrays of data points or objects with chart data |
| `ComparisonTable` | Arrays of objects (rows), ideal for side-by-side comparisons |
| `Timeline` | Ordered sequences of events |
| `ImageCard` | URLs or objects with image metadata |

**`BentoCard` shape:**

```ts
interface BentoCard {
  key:        string;                    // data_payload key, e.g. "cheapest_ev_usd"
  widget:     string;                    // "MetricCard" | "ChartCard" | ...
  props:      Record<string, unknown>;   // pass directly to your component
  isRefined?: boolean;                   // true when added/updated by refine()
}
```

---

## Advanced Topics

### Mission refinement

Refinement runs a second pass on an existing mission with a narrower intent. New results are merged into the original `data_payload` — keys that didn't exist before are added, keys that changed are updated. The `useOpalZero` hook marks changed cards `isRefined: true` so you can highlight them.

```tsx
// After a mission completes:
const { missionId, refine } = useOpalZero({ client });

// First run
await run("Compare the top 3 EVs under $60k");

// Then deepen — does not replace the existing card grid
await refine(missionId!, "Add charging network coverage data");
```

### Human-in-the-loop (HITL) feedback

Agents can call the built-in `feedback` tool to pause a mission and ask for human input. When this happens, the server emits a `mission_paused` event with a `question` string. Your application must call `POST /api/v1/clarify` with the answer to resume execution.

```ts
for await (const event of client.execute("Analyse this dataset")) {
  if (event.type === "mission_paused") {
    const answer = await promptUser(event.question); // your UI
    await fetch(`${baseUrl}/api/v1/clarify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ mission_id: event.mission_id, answer }),
    });
  }
}
```

### Uploading files for agent use

Upload a file before calling `execute()`. The server stores it and agents can reference it by the returned `filename`.

```ts
const result = await client.upload(myFile);
// result.filename is the server-side path agents can access

for await (const event of client.execute(
  `Analyse the uploaded CSV and find revenue trends — file: ${result.filename}`
)) { ... }
```

Supported file types: CSV, JSON, PDF, and images (PNG, JPEG, WebP). Maximum size: 10 MB.

### Checking server capabilities before rendering UI

```ts
const { openai, tavily, alpha_vantage } = await client.configStatus();

if (!tavily) {
  // hide "search the web" features in your UI
}
```

### Observing every event (tracing / analytics)

Use `onEvent` to tap the SSE stream for cross-cutting concerns without coupling them to component state:

```tsx
useOpalZero({
  client,
  onEvent(event) {
    analytics.track("opalzero_event", { type: event.type });
    if (event.type === "task_completed") {
      console.log("[trace]", event.slug, "→", event.result.slice(0, 120));
    }
  },
});
```

---

## TypeScript

All public types are exported from the main entry point:

```ts
import type {
  // Client
  OpalZeroClientConfig,

  // Mission lifecycle
  MissionStatus,      // "idle" | "running" | "complete" | "failed"
  MissionState,       // { data_payload, design_tokens?, layout_hint?, suggested_widgets? }
  MissionSnapshot,    // { mission_id, intent, plan, mission_state, status, created_at? }
  MissionSummary,     // { id, intent, status, created_at? }
  Task,               // { slug, role, intent, status, result? }

  // SSE events (full union)
  MissionEvent,
  TaskStartedEvent,
  TaskCompletedEvent,
  TaskFailedEvent,
  GovernorExpandEvent,
  MissionCompleteEvent,
  MissionFailedEvent,
  MissionPausedEvent,
  AwaitingFeedbackEvent,
  UnknownEvent,

  // Upload / config
  UploadResult,
  ConfigStatus,

  // Bento / React hook
  BentoCard,
  UseOpalZeroOptions,
  UseOpalZeroReturn,
} from "opal-zero";
```

---

## API Endpoints (HTTP reference)

For non-TypeScript clients or server-to-server use:

| Method | Path | Description |
|---|---|---|
| `POST` | `/api/v1/execute` | Start a mission; returns SSE stream |
| `POST` | `/api/v1/clarify` | Resume a paused mission with a human answer |
| `GET` | `/api/v1/missions` | List all past missions |
| `GET` | `/api/v1/missions/:id` | Get a mission snapshot |
| `DELETE` | `/api/v1/missions/:id` | Delete a mission |
| `POST` | `/api/v1/missions/:id/refine` | Refine a mission; returns SSE stream |
| `GET` | `/api/v1/missions/:id/export` | Export (`?format=md\|csv\|html`) |
| `POST` | `/api/v1/upload` | Upload a file (multipart/form-data) |
| `GET` | `/api/v1/config/status` | Check which API keys are configured |
| `GET` | `/health` | Health check |

**Auth header:** `X-OpalZero-Key: <your-key>` (only required when `OPALZERO_API_KEY` is set on the server).

**Per-request key overrides:**

| Header | Purpose |
|---|---|
| `X-OpenAI-Key` | Override the server's `OPENAI_API_KEY` for this request |
| `X-Tavily-Key` | Override the server's `TAVILY_API_KEY` for this request |
| `X-Alpha-Vantage-Key` | Override the server's `ALPHA_VANTAGE_API_KEY` for this request |

---

## License

MIT
