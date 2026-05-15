# axion-kernel

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)

**Premium OpenAI provider and structured Governor for production Axion deployments.**

axion-core ships with a built-in provider and a lightweight Governor that are great for development and self-hosted setups. axion-kernel upgrades both for production: a hardened OpenAI integration with vision support, and a Governor that injects the full mission context — all task results, the prior payload, and the ContextBus — into its quality evaluation prompt, making it significantly more accurate at catching coverage gaps and structural issues.

---

## What's included

### OpenAIProvider

Production-grade LLM provider backed by `gpt-4o-mini` for text tasks and `gpt-4o` for anything requiring vision. Configures timeouts, token limits, and temperature from environment variables — no code changes needed between environments. Implements the `AiProvider` trait from axion-core, so it's a drop-in replacement for `SimpleProvider`.

### AxionGovernor

An enhanced quality evaluator that goes beyond the built-in version in one key way: it injects the **complete ContextBus** — every agent's full output, not just a truncated preview — into its five-rubric evaluation prompt. This means the Governor catches cases where a WebSearcher found specific figures that the Analyst silently ignored, or where the final payload is structurally inconsistent with what earlier agents actually reported. It also includes a UI-nudge heuristic that detects data-rich missions missing a finalised dashboard state.

---

## Drop-in usage

axion-kernel activates automatically when `OPENAI_API_KEY` is present. Use it via axion-server (which imports it directly), or wire it into your own binary:

```rust
use axion_core::prelude::*;
use axion_kernel::prelude::{AxionGovernor, OpenAIProvider};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    axion_core::registry::Registry::init_default();

    // Drop-in replacements — same interface as SimpleProvider + BuiltinGovernor
    let provider = OpenAIProvider::new().expect("OPENAI_API_KEY must be set");
    let governor = AxionGovernor::new();

    let mut plan = build_plan_from_intent("Summarise the latest Rust 2024 edition changes", &provider).await;
    run_mission(&mut plan, &provider, &governor, 3, None).await.ok();
}
```

Add to `Cargo.toml`:

```toml
[dependencies]
axion-kernel = { git = "https://github.com/albertobarnabo/axion-kernel" }
axion-core   = { git = "https://github.com/albertobarnabo/axion-engine" }
tokio        = { version = "1", features = ["full"] }
dotenvy      = "0.15"
```

---

## Configuration

Inherits all env vars from axion-core. No additional variables required.

| Env var | Used by |
|---|---|
| `OPENAI_API_KEY` | OpenAIProvider (required) |
| `AXION_MAX_TOKENS` | Token limit per response |
| `AXION_TEMPERATURE` | Sampling temperature |
| `AXION_MISSION_TIMEOUT_SECS` | Wall-clock deadline |

---

## Docs

[albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs)

---

## License

MIT
