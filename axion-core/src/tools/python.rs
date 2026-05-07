/// Patterns that indicate destructive or privilege-escalating operations.
/// This is a deny-list filter only — Docker isolation is the real solution.
const BLOCKED: &[&str] = &[
    "rm -rf",
    "os.remove",
    "os.unlink",
    "os.rmdir",
    "shutil",
    "subprocess",
    "os.system",
    "os.popen",
    "os.execv",
    "os.execl",
    "__import__",
];

fn safety_check(code: &str) -> Result<(), String> {
    for pattern in BLOCKED {
        if code.contains(pattern) {
            return Err(format!(
                "Safety filter blocked execution: '{}' is not permitted. \
                 Use the calculator tool for arithmetic instead.",
                pattern
            ));
        }
    }
    Ok(())
}

/// Execute a snippet of Python code in a temporary file via `python3`.
/// Returns a Markdown-formatted string that includes the source code,
/// stdout output, and any stderr — ready for direct storage in the ContextBus.
pub fn execute_python(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct PyArgs {
        code: String,
    }

    let args: PyArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse python_interpreter arguments: {}", e))?;

    safety_check(&args.code)?;

    // Write to a uniquely named temp file so concurrent tasks don't collide.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp_path = std::env::temp_dir().join(format!("axion_{}.py", nonce));

    std::fs::write(&tmp_path, &args.code)
        .map_err(|e| format!("Failed to write temp script: {}", e))?;

    let output = std::process::Command::new("python3")
        .arg(&tmp_path)
        .output();

    // Always clean up, even if execution failed.
    let _ = std::fs::remove_file(&tmp_path);

    let output = output.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "python3 is not installed or not in PATH".to_string()
        } else {
            format!("Failed to launch python3: {}", e)
        }
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Build a Markdown block the UI can render and highlight.
    let mut result = format!("```python\n{}\n```\n\n", args.code.trim());

    if !stdout.is_empty() {
        result.push_str(&format!("**Output:**\n```\n{}\n```\n", stdout.trim()));
    }

    if !stderr.is_empty() {
        result.push_str(&format!("\n**Stderr:**\n```\n{}\n```\n", stderr.trim()));
    }

    if stdout.is_empty() && stderr.is_empty() {
        result.push_str("*(no output)*\n");
    }

    if !output.status.success() {
        return Err(format!(
            "Python exited with status {}.\n\nStderr:\n{}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(result)
}
