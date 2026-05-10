use std::path::Path;

use bytes::Bytes;
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::{
    pipe::{MemoryInputPipe, MemoryOutputPipe},
    preview1::{self, WasiP1Ctx},
    WasiCtxBuilder,
};

/// Sandboxed Wasm executor using WASI preview1.
///
/// Every call compiles and instantiates a fresh module — suitable for
/// tool-dispatch where invocations are infrequent.  The `Engine` (which
/// holds the JIT cache) could be lifted to a long-lived `OnceLock` later
/// without changing the public API.
///
/// # Isolation contract
/// - No filesystem access (directories not pre-opened).
/// - No network access.
/// - stdin  = `arguments` JSON string (UTF-8).
/// - stdout = captured in memory, returned as the tool result string.
/// - stderr = discarded (guest may write diagnostic text there freely).
pub struct WasmExecutor;

impl WasmExecutor {
    /// Load the `.wasm` file at `wasm_path`, feed `arguments` as stdin, and
    /// return the module's stdout as the tool result.
    pub fn run(wasm_path: &Path, arguments: &str) -> Result<String, String> {
        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("Failed to read {:?}: {}", wasm_path, e))?;
        Self::run_bytes(&wasm_bytes, arguments)
    }

    /// Execute already-loaded Wasm bytes (useful in tests that skip disk I/O).
    pub fn run_bytes(wasm_bytes: &[u8], arguments: &str) -> Result<String, String> {
        // ── Engine & module ────────────────────────────────────────────────
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| format!("Wasm compilation failed: {}", e))?;

        // ── Stdio plumbing ─────────────────────────────────────────────────
        // stdin : the JSON arguments string
        // stdout: captured in a cloneable in-memory pipe
        // stderr: inherited from the host process (writes appear in the Axion
        //         terminal log but are NOT returned as the tool result)
        let stdin = MemoryInputPipe::new(Bytes::copy_from_slice(arguments.as_bytes()));
        let stdout = MemoryOutputPipe::new(64 * 1024); // 64 KiB capture buffer
        let stdout_capture = stdout.clone(); // keeps a second Arc ref for post-run read

        // ── WASI context (preview1 / wasm32-wasip1) ────────────────────────
        // Restricted: no dirs pre-opened, no env vars, no args, only stdio.
        let wasi: WasiP1Ctx = WasiCtxBuilder::new()
            .stdin(stdin)
            .stdout(stdout)
            .build_p1();

        // ── Link WASI host functions into the module ───────────────────────
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        preview1::add_to_linker_sync(&mut linker, |ctx| ctx)
            .map_err(|e| format!("WASI linker setup failed: {}", e))?;

        // ── Instantiate and run ────────────────────────────────────────────
        let mut store = Store::new(&engine, wasi);

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("Wasm instantiation failed: {}", e))?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|e| format!("Wasm module has no '_start' export: {}", e))?;

        start
            .call(&mut store, ())
            .map_err(|e| format!("Wasm execution error: {}", e))?;

        // ── Capture stdout ─────────────────────────────────────────────────
        // Drop the store first so the WASI context releases its Arc ref to the
        // stdout pipe.  After the drop `stdout_capture` is the sole owner,
        // and `try_into_inner` can unwrap the Arc without allocating.
        drop(store);

        let bytes = stdout_capture
            .try_into_inner()
            .ok_or("stdout pipe still has live references after store drop")?;

        String::from_utf8(bytes.to_vec())
            .map_err(|e| format!("Wasm produced invalid UTF-8 output: {}", e))
    }
}
