//! `diff` tool — compute a readable text diff between two strings.
//!
//! Returns a unified diff plus a plain-English change summary (lines added,
//! removed, unchanged).  Use this in comparison missions: "what changed between
//! this week's pricing page and last week's", "how does version A differ from
//! version B", etc.

use similar::{ChangeTag, TextDiff};
use serde::{Deserialize, Serialize};

const MAX_INPUT_CHARS: usize = 32_000;
const MAX_DIFF_CHARS:  usize = 16_000;

#[derive(Deserialize)]
struct DiffArgs {
    /// The original (before) text.
    original: String,
    /// The modified (after) text.
    modified: String,
    /// Number of unchanged lines of context around each change (default: 3).
    #[serde(default = "default_context")]
    context_lines: usize,
}

fn default_context() -> usize { 3 }

#[derive(Serialize)]
struct DiffResult {
    lines_added:   usize,
    lines_removed: usize,
    lines_unchanged: usize,
    has_changes:   bool,
    /// Unified diff in standard format — suitable for display or further analysis.
    unified_diff:  String,
    truncated:     bool,
}

pub fn execute_diff(arguments: &str) -> Result<String, String> {
    let args: DiffArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("diff: invalid arguments: {e}"))?;

    // Enforce input size limit to keep context budget sane.
    if args.original.len() > MAX_INPUT_CHARS || args.modified.len() > MAX_INPUT_CHARS {
        return Err(format!(
            "diff: inputs must be under {} characters each (original={}, modified={})",
            MAX_INPUT_CHARS, args.original.len(), args.modified.len()
        ));
    }

    let td = TextDiff::from_lines(&args.original, &args.modified);

    let mut lines_added   = 0usize;
    let mut lines_removed = 0usize;
    let mut lines_unchanged = 0usize;

    for change in td.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => lines_added   += 1,
            ChangeTag::Delete => lines_removed += 1,
            ChangeTag::Equal  => lines_unchanged += 1,
        }
    }

    // Build unified diff string.
    let mut diff_str = String::new();
    for group in td.grouped_ops(args.context_lines) {
        for op in &group {
            for change in td.iter_changes(op) {
                let prefix = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal  => " ",
                };
                diff_str.push_str(prefix);
                diff_str.push_str(change.value());
                if change.missing_newline() {
                    diff_str.push('\n');
                }
            }
        }
        diff_str.push('\n');
    }

    let truncated = diff_str.len() > MAX_DIFF_CHARS;
    if truncated {
        diff_str.truncate(MAX_DIFF_CHARS);
        diff_str.push_str("\n… [diff truncated]");
    }

    let result = DiffResult {
        lines_added,
        lines_removed,
        lines_unchanged,
        has_changes: lines_added > 0 || lines_removed > 0,
        unified_diff: diff_str,
        truncated,
    };
    serde_json::to_string(&result)
        .map_err(|e| format!("diff: serialisation failed: {e}"))
}
