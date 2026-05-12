//! Axion Vision — Wasm Professional
//!
//! Compiled to `wasm32-wasip1`.  The host exposes `/uploads` (read-only) via
//! a WASI preopen.  The tool reads the requested image file, base64-encodes
//! it, and returns a JSON proxy payload that the Axion Kernel forwards to a
//! vision-capable LLM.
//!
//! Protocol:
//!   stdin  → JSON: `{"file_path":"chart.png","prompt":"optional question"}`
//!   stdout → JSON: `{"image_base64":"…","mime_type":"image/png","file_path":"chart.png"}`
//!   stderr → error message + exit(1) on failure

use std::fs;
use std::io::Read;
use std::path::Path;

// ── Input / Output ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct VisionArgs {
    file_path: String,
    #[serde(default)]
    prompt: Option<String>,
}

#[derive(serde::Serialize)]
struct VisionProxy {
    image_base64: String,
    mime_type: String,
    file_path: String,
    /// Echo the prompt so the host can forward it to the vision LLM.
    prompt: String,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

const UPLOADS_DIR: &str = "/uploads";

fn mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// RFC 4648 standard base64 encoder (no external crate needed in Wasm).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { CHARS[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { CHARS[(n & 63) as usize] as char } else { '=' });
    }
    out
}

// ── Core logic ─────────────────────────────────────────────────────────────────

fn prepare(args: &VisionArgs) -> Result<VisionProxy, String> {
    // Sanitize: reject path traversal attempts.
    let name = args.file_path.trim_start_matches('/');
    if name.contains("..") || name.contains('\\') || name.is_empty() {
        return Err(format!("Invalid file_path: '{}'", args.file_path));
    }

    let path = Path::new(UPLOADS_DIR).join(name);
    let bytes = fs::read(&path)
        .map_err(|e| format!("Cannot read {:?}: {}", path, e))?;

    let mime = mime_from_path(&path);
    let prompt = args.prompt.clone().unwrap_or_else(|| {
        "Describe this image in detail. \
         If it contains data (chart, table, graph, text), extract and summarise the key values."
            .to_string()
    });

    Ok(VisionProxy {
        image_base64: base64_encode(&bytes),
        mime_type: mime.to_string(),
        file_path: name.to_string(),
        prompt,
    })
}

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("Failed to read stdin");

    let args: VisionArgs = match serde_json::from_str(input.trim()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Vision tool: failed to parse arguments: {}", e);
            std::process::exit(1);
        }
    };

    match prepare(&args) {
        Ok(proxy) => print!("{}", serde_json::to_string(&proxy).expect("serialize")),
        Err(e) => {
            eprintln!("Vision tool error: {}", e);
            std::process::exit(1);
        }
    }
}
