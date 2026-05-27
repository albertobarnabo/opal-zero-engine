//! `extract_pdf_text` tool — extract readable text from a PDF file.
//!
//! Reads from the `uploads/` directory (same as `read_file`).
//! Returns the extracted text, page count, and character count.
//! Output is capped at 32 KB; if the PDF is longer the agent receives the
//! first portion plus a truncation notice.
//!
//! Requires `OPALZERO_UPLOAD_DIR` or defaults to `uploads/`.

use serde::{Deserialize, Serialize};

const MAX_OUTPUT_CHARS: usize = 32_768;

#[derive(Deserialize)]
struct PdfArgs {
    /// Filename inside the `uploads/` directory (e.g. `"report.pdf"`).
    filename: String,
}

#[derive(Serialize)]
struct PdfResult {
    filename:   String,
    page_count: usize,
    char_count: usize,
    text:       String,
    truncated:  bool,
}

pub fn execute_extract_pdf_text(arguments: &str) -> Result<String, String> {
    let args: PdfArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("extract_pdf_text: invalid arguments: {e}"))?;

    let name = args.filename.trim();
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(format!("extract_pdf_text: invalid filename '{name}'"));
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext != "pdf" {
        return Err(format!("extract_pdf_text: expected a .pdf file (got '.{ext}')"));
    }

    let path = std::path::Path::new("uploads").join(name);
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("extract_pdf_text: cannot read '{name}': {e}"))?;

    // pdf-extract uses lopdf under the hood — pure Rust, no system deps.
    let text = pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| format!("extract_pdf_text: extraction failed: {e}"))?;

    // Rough page count via form-feed characters or trailer info.
    let page_count = text.chars().filter(|&c| c == '\x0C').count().max(1);
    let char_count = text.chars().count();

    let (out, truncated) = if char_count > MAX_OUTPUT_CHARS {
        let end = (0..=MAX_OUTPUT_CHARS.min(text.len())).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
        (
            format!("{}… [truncated — {} total chars across ~{} pages]",
                &text[..end], char_count, page_count),
            true,
        )
    } else {
        (text, false)
    };

    let result = PdfResult {
        filename:   name.to_string(),
        page_count,
        char_count,
        text:       out,
        truncated,
    };
    serde_json::to_string(&result)
        .map_err(|e| format!("extract_pdf_text: serialisation failed: {e}"))
}
