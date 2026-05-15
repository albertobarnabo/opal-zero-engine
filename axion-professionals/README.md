# axion-professionals

The WASM tool registry for Axion. Each Professional is a Rust crate that compiles to WebAssembly and extends what Axion agents can do at runtime — without modifying the core engine.

## Built-in Professionals

| Tool | Description |
|---|---|
| `calculator` | Safe arithmetic expression evaluator |
| `memory` | Per-mission key/value store |
| `memory_persist` | Cross-mission persistent memory (writes to `memory/global.json`) |
| `web_search` | Web search via Tavily API |
| `vision` | Image analysis via GPT-4o vision |
| `python_interpreter` | Sandboxed Python 3 execution |
| `feedback` | Human-in-the-loop pause/resume |
| `generate_document` | Export mission findings as MD/CSV/HTML |
| `read_file` | Read uploaded data files (CSV, JSON, TXT) |

## Build a Professional

See the full contributor guide at [albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs).

Quick start:

```rust
use axion_sdk::{Professional, Context, Output};

#[derive(Default)]
pub struct MyProfessional;

impl Professional for MyProfessional {
    fn name(&self) -> &'static str { "my_tool" }

    async fn run(&self, ctx: Context) -> Result<Output> {
        Ok(Output::text("Hello from my tool!"))
    }
}

axion_sdk::export_professional!(MyProfessional);
```

```bash
wasm-pack build --target bundler --out-dir pkg
# Drop the .wasm into registry/ — the kernel picks it up on next start
```

## License

MIT
