# axion-core

The Rust brain of the Axion Intelligence Kernel. Handles mission orchestration, agent dispatch, quality validation, and tool execution.

## What it does

axion-core takes a user intent, breaks it into tasks via a Planner agent, dispatches each task to a specialist agent (Analyst, Coder, WebSearcher), validates the combined output through a Governor with structured quality rubrics, and streams results as `MissionUpdate` events.

## Architecture

```
Intent → Planner → [Analyst | Coder | WebSearcher] → Governor → MissionComplete
                        ↑                    ↓
                   ContextBus ←──── cross-mission memory ─┘
```

## Key features

- **Task dependency graph** — tasks declare `depends_on` slugs; the dispatcher honours ordering and cascade-fails blocked tasks
- **Structured Governor rubrics** — five-criterion quality evaluation (intent coverage, data density, structural validity, claim specificity, synthesis completeness)
- **Engine retry** — 3-attempt exponential backoff on transient LLM errors
- **Cross-mission persistent memory** — facts written by the Analyst/Planner survive between missions
- **Smart context windowing** — 6000-char budget, per-entry cap, prevents prompt bloat
- **Mission timeout** — configurable wall-clock deadline, emits `MissionFailed` on breach

## Usage

axion-core is a library crate. Use it via [axion-server](https://github.com/albertobarnabo/axion-lab) (the HTTP layer) or call it directly:

```rust
axion_core::run_mission("Research the best EVs under $60k", tx).await;
```

## Configuration

| Env var | Default | Description |
|---|---|---|
| `OPENAI_API_KEY` | required | LLM provider key |
| `AXION_MAX_TOKENS` | 4096 | Max tokens per response |
| `AXION_TEMPERATURE` | 0.1 | Sampling temperature |
| `AXION_MISSION_TIMEOUT_SECS` | 300 | Mission wall-clock limit |
| `AXION_MAX_LLM_RETRIES` | 3 | Retry attempts on LLM errors |

## Docs

[albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs)

## License

MIT
