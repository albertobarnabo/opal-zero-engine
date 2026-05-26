# opalzero-professionals

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)

**The WASM tool registry that gives OpalZero agents their superpowers.**

Professionals are Rust crates that compile to WebAssembly and extend what OpalZero agents can do at runtime — without touching the core engine. Drop a `.wasm` binary into the registry directory and the kernel picks it up on next start. No recompilation, no config changes, no restarts beyond the initial load.

---

## Built-in tools

| # | Tool | What it does |
|---|---|---|
| 🧮 | `calculator` | Safe arithmetic expression evaluator — agents use it instead of hallucinating math |
| 🗂️ | `memory` | Per-mission key/value store — read and write context within a single mission |
| 🧠 | `memory_persist` | Cross-mission persistent memory — writes to `memory/global.json`, survives restarts |
| 🔍 | `web_search` | Live web search via Tavily API — returns ranked results with source URLs |
| 👁️ | `vision` | Image analysis via GPT-4o vision — works on uploads and URLs |
| 🐍 | `python_interpreter` | Sandboxed Python 3 execution — runs agent-generated code safely |
| 🙋 | `feedback` | Human-in-the-loop pause/resume — agent asks a question, mission waits for your answer |
| 📄 | `generate_document` | Export mission findings as MD, CSV, or styled HTML |
| 📂 | `read_file` | Read uploaded data files (CSV, JSON, TXT) into agent context |

---

## Build your own in 3 steps

**1. Implement the `Professional` trait**

```rust
use opalzero_sdk::{Professional, Context, Output};

#[derive(Default)]
pub struct CurrencyConverter;

impl Professional for CurrencyConverter {
    fn name(&self) -> &'static str { "currency_convert" }

    async fn run(&self, ctx: Context) -> Result<Output> {
        let amount: f64 = ctx.get("amount")?;
        let rate:   f64 = ctx.get("rate")?;
        Ok(Output::text(format!("{:.2}", amount * rate)))
    }
}

opalzero_sdk::export_professional!(CurrencyConverter);
```

**2. Compile to WASM**

```bash
wasm-pack build --target bundler --out-dir pkg
```

**3. Drop it in the registry**

```bash
cp pkg/currency_converter_bg.wasm /path/to/opalzero/registry/
# The kernel loads it on next start — agents can call "currency_convert" immediately
```

---

## Contributing

Full contributor guide — manifest format, tool schema, testing — at [albertobarnabo.it/opalzero/docs](https://albertobarnabo.it/opalzero/docs).

---

## License

MIT

⭐ Built something useful? Open a PR — community Professionals are welcome.
