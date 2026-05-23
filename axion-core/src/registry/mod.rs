use crate::protocol::Tool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ── Global registry ───────────────────────────────────────────────────────────

static GLOBAL: OnceLock<Registry> = OnceLock::new();

// ── Registry struct ───────────────────────────────────────────────────────────

/// An immutable, lazily-initialized map of tool name → `Tool` definition.
///
/// Loaded from the `professionals/manifests/*.json` directory at startup via
/// [`Registry::init_default`].  Falls back gracefully to the hard-coded
/// constructors on `Tool` when the registry has not been initialised
/// (e.g. during unit tests).
pub struct Registry {
    tools: HashMap<String, Tool>,
    /// Parent of the `manifests/` directory — i.e. the `professionals/` folder.
    /// Used by the executor to locate `.wasm` binaries alongside the manifests.
    professionals_dir: Option<PathBuf>,
}

impl Registry {
    // ── Initialisation ────────────────────────────────────────────────────────

    /// Scan `dir` for `*.json` manifest files, parse each as a [`Tool`], and
    /// store the result in the global `OnceLock`.
    ///
    /// Silently skips files that cannot be parsed so a single bad manifest
    /// never takes down the whole registry.
    pub fn init(dir: &Path) {
        let registry = Self::load(dir).unwrap_or_else(|e| {
            tracing::warn!(dir = ?dir, error = %e, "Registry::init: could not load manifests");
            Registry { tools: HashMap::new(), professionals_dir: None }
        });
        // `set` is a no-op if already initialised — safe to call multiple times.
        let _ = GLOBAL.set(registry);
    }

    /// Try multiple strategies to locate manifests:
    /// 1. Compile-time manifest path (for axion-core)
    /// 2. Sibling axion-core path (when called from axion-server)
    /// 3. CWD-relative path (fallback for other environments)
    pub fn init_default() {
        let compile_time_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/professionals/manifests"
        );

        // Strategy 1: Try compile-time path
        if Path::new(compile_time_path).is_dir() {
            return Self::init(Path::new(compile_time_path));
        }

        // Strategy 2: If in axion-server, look for axion-core/professionals/manifests
        // CARGO_MANIFEST_DIR will be .../axion-server, so go up and into axion-core
        if compile_time_path.contains("axion-server") {
            let manifest_dir = PathBuf::from(compile_time_path)
                .parent()
                .and_then(|p| p.parent()) // Go to workspace root
                .map(|p| p.join("axion-core/professionals/manifests"));
            
            if let Some(path) = manifest_dir {
                if path.is_dir() {
                    return Self::init(&path);
                }
            }
        }

        // Strategy 3: CWD-relative fallback
        Self::init(Path::new("professionals/manifests"));
    }

    // ── Internal loader ───────────────────────────────────────────────────────

    fn load(dir: &Path) -> Result<Self, String> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| format!("Failed to read manifests directory {:?}: {}", dir, e))?;

        let mut tools = HashMap::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match Tool::from_manifest(&path) {
                Ok(tool) => {
                    tracing::debug!(tool_name = %tool.name, "Registry: loaded manifest");
                    tools.insert(tool.name.clone(), tool);
                }
                Err(e) => {
                    tracing::warn!(path = ?path, error = %e, "Registry: skipping manifest");
                }
            }
        }

        // The professionals/ directory is the parent of manifests/.
        let professionals_dir = dir.parent().map(PathBuf::from);

        tracing::info!(tool_count = tools.len(), "Registry: tools loaded");
        Ok(Registry { tools, professionals_dir })
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Return the global registry if it has been initialised, or `None`.
    pub fn get() -> Option<&'static Registry> {
        GLOBAL.get()
    }

    /// Look up a tool by name.
    pub fn get_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    /// Return cloned `Tool` objects for the given list of names.
    /// Names that are not in the registry are silently skipped.
    pub fn tools_for_names(&self, names: &[&str]) -> Vec<Tool> {
        names
            .iter()
            .filter_map(|name| self.tools.get(*name).cloned())
            .collect()
    }

    /// Return `true` if `name` is a known registered tool.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Return the `professionals/` directory that manifests were loaded from,
    /// if available.  `.wasm` binaries are expected alongside this directory.
    pub fn professionals_dir(&self) -> Option<&Path> {
        self.professionals_dir.as_deref()
    }

    /// Construct the expected `.wasm` path for `tool_name` — i.e.
    /// `professionals/{tool_name}.wasm`.  Returns `None` if the professionals
    /// directory was not recorded during init.
    pub fn wasm_path_for(&self, tool_name: &str) -> Option<PathBuf> {
        self.professionals_dir
            .as_ref()
            .map(|dir| dir.join(format!("{}.wasm", tool_name)))
    }
}
