//! Axion Feedback — Wasm Professional
//!
//! A "virtual" tool: it performs no computation but signals the Axion Kernel
//! that the mission should pause and wait for human input.
//!
//! The Kernel detects the `__AWAITING_FEEDBACK__: ` prefix in the tool output
//! and transitions the mission to the `AwaitingFeedback` state.
//!
//! Compiled to `wasm32-wasip1`; no filesystem preopens required.
//!
//! Protocol:
//!   stdin  → JSON: `{"question":"…","context":"…"}`
//!   stdout → `"__AWAITING_FEEDBACK__: <question>"`
//!   stderr → error message + exit(1) on failure

use std::io::Read;

const AWAITING_FEEDBACK_PREFIX: &str = "__AWAITING_FEEDBACK__: ";

#[derive(serde::Deserialize)]
struct FeedbackArgs {
    question: String,
    #[serde(default)]
    context: String,
}

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("Failed to read stdin");

    let args: FeedbackArgs = match serde_json::from_str(input.trim()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Feedback tool: failed to parse arguments: {}", e);
            std::process::exit(1);
        }
    };

    if args.question.trim().is_empty() {
        eprintln!("Feedback tool: 'question' must not be empty");
        std::process::exit(1);
    }

    // The context block is appended after a newline so the Kernel can strip the
    // prefix to recover the bare question on the first line.
    let output = if args.context.is_empty() {
        format!("{}{}", AWAITING_FEEDBACK_PREFIX, args.question.trim())
    } else {
        format!(
            "{}{}\n\nContext: {}",
            AWAITING_FEEDBACK_PREFIX,
            args.question.trim(),
            args.context.trim()
        )
    };

    print!("{}", output);
}
