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
    let tmp_path = std::env::temp_dir().join(format!("opalzero_{}.py", nonce));

    std::fs::write(&tmp_path, &args.code)
        .map_err(|e| format!("Failed to write temp script: {}", e))?;

    // Wrap execution in an AST-aware runner so that bare expressions on the
    // last line (e.g. `1 + 1`, `result`, `df.head()`) are automatically
    // printed — matching Python REPL / Jupyter notebook behaviour.
    let runner_path = std::env::temp_dir().join(format!("opalzero_{}_run.py", nonce));
    let runner_script = format!(
        "import ast as _a, sys as _s\n\
         with open(r'{}') as _f:\n\
         \t_c = _f.read()\n\
         _t = _a.parse(_c)\n\
         _ns = {{'__name__': '__main__'}}\n\
         if _t.body and isinstance(_t.body[-1], _a.Expr):\n\
         \t_l = _t.body.pop()\n\
         \texec(compile(_t, '<opalzero>', 'exec'), _ns)\n\
         \t_v = eval(compile(_a.Expression(body=_l.value), '<opalzero>', 'eval'), _ns)\n\
         \tif _v is not None:\n\
         \t\tprint(repr(_v))\n\
         else:\n\
         \texec(compile(_t, '<opalzero>', 'exec'), _ns)\n",
        tmp_path.to_string_lossy()
    );
    std::fs::write(&runner_path, &runner_script)
        .map_err(|e| format!("Failed to write runner script: {}", e))?;

    let output = std::process::Command::new("python3")
        .arg(&runner_path)
        .output();

    // Always clean up both files, even if execution failed.
    let _ = std::fs::remove_file(&tmp_path);
    let _ = std::fs::remove_file(&runner_path);

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
    use std::sync::Mutex;

    // Any test that actually invokes python3 and creates a temp file must hold
    // this lock for the duration of the test.  Concurrent tests that write
    // opalzero_*.py files would otherwise corrupt the before/after snapshot
    // comparison in `execute_python_cleans_up_temp_file`.
    //
    // Tests that fail before a file is created (JSON errors, safety-check
    // rejections) do NOT need the lock — they never touch the temp dir.
    static EXEC_LOCK: Mutex<()> = Mutex::new(());

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
        assert!(safety_check("s = 'hello opalzero'\nprint(s.upper())").is_ok());
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
    fn execute_python_no_output_marker_for_pure_assignment() {
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");
        // An assignment statement has no expression value — still produces no output.
        let json = r#"{"code": "x = 42"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("*(no output)*"), "pure assignment should still give no-output");
        assert!(result.contains("```python"));
    }

    #[test]
    fn execute_python_auto_prints_last_expression() {
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");
        // A bare expression on the last line should be auto-printed (REPL behaviour).
        let json = r#"{"code": "x = 42\nx"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("42"), "bare name expression should auto-print its value");
        assert!(!result.contains("*(no output)*"));
    }

    #[test]
    fn execute_python_auto_prints_arithmetic_expression() {
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");
        let json = r#"{"code": "300 + 240"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("540"), "arithmetic expression should auto-print result");
    }

    #[test]
    fn execute_python_captures_stdout() {
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");
        let json = r#"{"code": "print('hello opalzero')"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("hello opalzero"));
        assert!(result.contains("**Output:**"));
        assert!(result.contains("```python"));
    }

    #[test]
    fn execute_python_output_contains_source_block() {
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");
        let json = r#"{"code": "print(6 * 7)"}"#;
        let result = execute_python(json).unwrap();
        assert!(result.contains("```python"), "Output should include a fenced code block");
        assert!(result.contains("42"), "Output should contain the computed result");
    }

    #[test]
    fn execute_python_cleans_up_temp_file() {
        use std::collections::HashSet;
        use std::path::PathBuf;

        // Hold the lock for the entire test so no sibling test can create a
        // competing opalzero_*.py file between the before/after snapshots.
        let _guard = EXEC_LOCK.lock().expect("EXEC_LOCK poisoned");

        let snapshot = |dir: &std::path::Path| -> HashSet<PathBuf> {
            std::fs::read_dir(dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("opalzero_"))
                .map(|e| e.path())
                .collect()
        };

        let tmp = std::env::temp_dir();
        let before = snapshot(&tmp);
        let _ = execute_python(r#"{"code": "x = 1"}"#);
        let after = snapshot(&tmp);

        // Any opalzero_* file in `after` but not in `before` is a leak (both the
        // user script and the runner wrapper must be cleaned up).
        let leaked: Vec<_> = after.difference(&before).collect();
        assert!(
            leaked.is_empty(),
            "execute_python left behind temp file(s): {:?}",
            leaked
        );
    }
}
