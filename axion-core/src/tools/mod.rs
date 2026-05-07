pub async fn execute_tool(name: &str, arguments: &str) -> Result<String, String> {
    match name {
        "calculator" => execute_calculator(arguments),
        "web_search" => execute_web_search(arguments).await,
        "write_file" => execute_write_file(arguments),
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn execute_calculator(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct CalcArgs {
        operation: String,
        #[serde(default)]
        values: Vec<f64>,
    }

    let args: CalcArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse calculator arguments: {}", e))?;

    if args.values.is_empty() {
        return Err("No values provided for calculation".to_string());
    }

    let result = match args.operation.as_str() {
        "add" => args.values.iter().sum::<f64>(),
        "subtract" => args.values[1..].iter().fold(args.values[0], |acc, &x| acc - x),
        "multiply" => args.values.iter().fold(1.0, |acc, &x| acc * x),
        "divide" => {
            let mut result = args.values[0];
            for &val in &args.values[1..] {
                if val == 0.0 {
                    return Err("Division by zero".to_string());
                }
                result /= val;
            }
            result
        }
        _ => return Err(format!("Unknown operation: {}", args.operation)),
    };

    Ok(format!("Result: {}", result))
}

fn execute_write_file(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct WriteArgs {
        filename: String,
        content: String,
    }

    let args: WriteArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse write_file arguments: {}", e))?;

    // Safety: reject filenames with path separators or traversal sequences.
    if args.filename.contains('/') || args.filename.contains('\\') || args.filename.contains("..") {
        return Err(format!(
            "Invalid filename '{}': must not contain path separators or '..'",
            args.filename
        ));
    }

    let output_dir = std::path::Path::new("output");
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output/ directory: {}", e))?;

    let path = output_dir.join(&args.filename);
    std::fs::write(&path, &args.content)
        .map_err(|e| format!("Failed to write '{}': {}", args.filename, e))?;

    Ok(format!("File 'output/{}' written successfully.", args.filename))
}

async fn execute_web_search(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct SearchArgs {
        query: String,
    }

    let args: SearchArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse search arguments: {}", e))?;

    Ok(format!(
        "Web search results for '{}': [Simulated results showing top 3 results for the query]",
        args.query
    ))
}
