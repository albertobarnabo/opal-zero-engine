# Tool manifests

This directory is the **runtime manifest registry** read by the OpalZero kernel
at startup.  It contains two kinds of files:

## Auto-generated (source lives in `opalzero-professionals/`)

These files are **copied here by the build step** — do not edit them directly.
Edit the `manifest.json` inside the corresponding professional's source directory
and run `opalzero-professionals/build.sh` to propagate the change here.

| File | Source |
|---|---|
| `calculator.json` | `opalzero-professionals/calculator/manifest.json` |
| `memory.json` | `opalzero-professionals/memory/manifest.json` |
| `vision.json` | `opalzero-professionals/vision/manifest.json` |
| `feedback.json` | `opalzero-professionals/feedback/manifest.json` |

## Native-only (source lives in `opalzero-core/src/tools/`)

These tools are implemented as native Rust in `opalzero-core` and have no WASM
binary.  This directory is their single source of truth — edit here directly.

`finalize_mission_state.json`, `generate_document.json`, `get_company_overview.json`,
`get_income_statement.json`, `get_news_sentiment.json`, `get_price_history.json`,
`http_request.json` *(if present)*, `memory_persist.json`, `python_interpreter.json`,
`read_file.json`, `web_search.json`, `write_file.json`
