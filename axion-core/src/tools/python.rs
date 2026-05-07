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

#[cfg(test)]
mod tests {
    use super::*;

    // ── safety_check ────────────────────────────────────────────────────────

    #[test]
    fn safety_check_blocks_every_denied_pattern() {
        for &pattern in BLOCKED {
            let code = format!("x = 1\n{}\ny = 2", pattern);
            let result = safety_check(&code);
            assert!(result.is_err(), "Expected '{}' to be blocked", pattern);
            assert!(
                result.unwrap_err().contains("Safety filter"),
                "Error message should mention 'Safety filter' for pattern '{}'",
                pattern
            );
        }
    }

    #[test]
    fn safety_check_allows_safe_arithmetic() {
        assert!(safety_check("x = 1 + 2\nprint(x)").is_ok());
    }

    #[test]
    fn safety_check_allows_stdlib_import() {
        assert!(safety_check("import math\nprint(math.sqrt(16))").is_ok());
    }

    #[test]
    fn safety_check_allows_string_manipulation() {
        assert!(safety_check("s = 'hello axion'\nprint(s.upper())").is_ok());
    }

    // ── execute_python ───────────────────────────────────────────────────────

    #[test]
    fn execute_python_rejects_invalid_json() {
        let err = execute_python("not json").unwrap_err();
        assert!(err.contains("Failed to parse python_interpreter arguments"));
    }

    #[test]
    fn execute_python_rejects_missing_code_key() {
        let err = execute_python(r#"{"script": "print(1)"}"#).unwrap_err();
        assert!(err.contains("Failed to parse"));
    }

    #[test]
    fn execute_python_blocks_denied_code() {
        let json = r#"{"code": "import subprocess\nsubprocess.run(['ls'])"}"#;
        let err = execute_python(json).unwrap_err();
        assert!(err.contains("Safety filter"));
    }

    #[test]
    fn execute_python_no_output_marker() {
        // Code that runs but produces no stdout or stderr.
        let json = r#"{"code": "x = 42"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("*(no output)*"));
        assert!(result.contains("```python"));
    }

    #[test]
    fn execute_python_captures_stdout() {
        let json = r#"{"code": "print('hello axion')"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("hello axion"));
        assert!(result.contains("**Output:**"));
        assert!(result.contains("```python"));
    }

    #[test]
    fn execute_python_output_contains_source_block() {
        let json = r#"{"code": "print(6 * 7)"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("```python"), "Output should include a fenced code block");
        assert!(result.contains("42"), "Output should contain the computed result");
    }

    #[test]
    fn execute_python_cleans_up_temp_file() {
        use std::collections::HashSet;
        use std::path::PathBuf;

        let snapshot = |dir: &std::path::Path| -> HashSet<PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("axion_"))
                .map(|e| e.path())
                .collect()
        };

        let tmp = std::env::temp_dir();
        let before = snapshot(&tmp);
        let _ = execute_python(r#"{"code": "x = 1"}"#);
        let after = snapshot(&tmp);

        // Any file in `after` but not in `before` is a leak from this call.
        // Files that disappeared between snapshots are from concurrent tests (fine).
        let leaked: Vec<_> = after.difference(&before).collect();
        assert!(
            leaked.is_empty(),
            "execute_python left behind temp file(s): {:?}",
            leaked
        );
    }
}
