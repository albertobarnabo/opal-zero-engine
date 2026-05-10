//! Axion Calculator — Wasm Professional
//!
//! Compiled to `wasm32-wasip1` and executed inside the Axion `WasmExecutor`.
//!
//! Protocol:
//!   stdin  → JSON: `{"operation": "add|subtract|multiply|divide", "values": [f64...]}`
//!   stdout → `"Result: <number>"` on success
//!   stderr → error message + exit(1) on failure

use std::io::Read;

#[derive(serde::Deserialize)]
struct CalcArgs {
    operation: String,
    #[serde(default)]
    values: Vec<f64>,
}

fn compute(input: &str) -> Result<String, String> {
    let args: CalcArgs = serde_json::from_str(input)
        .map_err(|e| format!("Failed to parse calculator arguments: {}", e))?;

    if args.values.is_empty() {
        return Err("No values provided for calculation".to_string());
    }

    let result = match args.operation.as_str() {
        "add" => args.values.iter().sum::<f64>(),
        "subtract" => args.values[1..].iter().fold(args.values[0], |acc, &x| acc - x),
        "multiply" => args.values.iter().fold(1.0, |acc, &x| acc * x),
        "divide" => {
            let mut r = args.values[0];
            for &v in &args.values[1..] {
                if v == 0.0 {
                    return Err("Division by zero".to_string());
                }
                r /= v;
            }
            r
        }
        _ => return Err(format!("Unknown operation: '{}'", args.operation)),
    };

    Ok(format!("Result: {}", result))
}

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("Failed to read stdin");

    match compute(input.trim()) {
        Ok(result) => print!("{}", result),
        Err(e) => {
            eprintln!("Calculator error: {}", e);
            std::process::exit(1);
        }
    }
}
