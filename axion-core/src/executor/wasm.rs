use std::path::Path;

use bytes::Bytes;
use wasmtime::{Engine, Linker, Module, Store};
use wasmtime_wasi::{
    pipe::{MemoryInputPipe, MemoryOutputPipe},
    preview1::{self, WasiP1Ctx},
    DirPerms, FilePerms, WasiCtxBuilder,
};

use crate::protocol::WasmPreopen;

/// Sandboxed Wasm executor using WASI preview1.
///
/// Every call compiles and instantiates a fresh module — suitable for
/// tool-dispatch where invocations are infrequent.
///
/// # Isolation contract
/// - Filesystem: only directories explicitly listed in `preopens` are visible
///   to the guest, and only with the permissions declared in each entry.
/// - No network access.
/// - stdin  = `arguments` JSON string (UTF-8).
/// - stdout = captured in memory, returned as the tool result string.
/// - stderr = printed to the Axion terminal for diagnostics; NOT returned.
pub struct WasmExecutor;

impl WasmExecutor {
    /// Load the `.wasm` file at `wasm_path`, feed `arguments` as stdin,
    /// pre-open the host directories in `preopens`, and return stdout.
    pub fn run(
        wasm_path: &Path,
        arguments: &str,
        preopens: &[WasmPreopen],
    ) -> Result<String, String> {
        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("Failed to read {:?}: {}", wasm_path, e))?;
        Self::run_bytes(&wasm_bytes, arguments, preopens)
    }

    /// Execute already-loaded Wasm bytes (useful in tests that skip disk I/O).
    pub fn run_bytes(
        wasm_bytes: &[u8],
        arguments: &str,
        preopens: &[WasmPreopen],
    ) -> Result<String, String> {
        // ── Engine & module ────────────────────────────────────────────────
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| format!("Wasm compilation failed: {}", e))?;

        // ── Stdio plumbing ─────────────────────────────────────────────────
        let stdin = MemoryInputPipe::new(Bytes::copy_from_slice(arguments.as_bytes()));
        let stdout = MemoryOutputPipe::new(256 * 1024); // 256 KiB — snapshots can be large
        let stdout_capture = stdout.clone();

        // ── WASI context ───────────────────────────────────────────────────
        let mut builder = WasiCtxBuilder::new();
        builder.stdin(stdin).stdout(stdout);

        // Wire up any host directories declared by this tool's manifest.
        // `preopened_dir` calls `cap_std::Dir::open_ambient_dir` internally,
        // so we only need the host path string and the desired permissions.
        for p in preopens {
            let host_path = Path::new(&p.host);

            if !host_path.exists() {
                // Directory absent (e.g. `missions/` before the first save).
                // Skip silently — the guest handles the missing path itself.
                tracing::warn!(host_path = ?host_path, "WasmExecutor: preopen host path not found — skipping");
                continue;
            }

            let (dp, fp) = if p.readonly {
                (DirPerms::READ, FilePerms::READ)
            } else {
                (DirPerms::all(), FilePerms::all())
            };

            builder
                .preopened_dir(host_path, &p.guest, dp, fp)
                .map_err(|e| format!("Failed to preopen {:?}: {}", host_path, e))?;
        }

        let wasi: WasiP1Ctx = builder.build_p1();

        // ── Link WASI host functions ───────────────────────────────────────
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

        // Drop store so the WASI context releases its Arc ref to the stdout pipe.
        drop(store);

        let bytes = stdout_capture
            .try_into_inner()
            .ok_or("stdout pipe still has live references after store drop")?;

        String::from_utf8(bytes.to_vec())
            .map_err(|e| format!("Wasm produced invalid UTF-8 output: {}", e))
    }
}
