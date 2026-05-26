//! `read_csv` tool — parse a CSV/TSV file into structured JSON.
//!
//! Unlike `read_file`, which dumps raw bytes, this tool parses the file into
//! a JSON array of objects keyed by column header.  Agents can reason about
//! individual columns, combine with `sqlite_query` for analysis, or reference
//! specific cells by name.
//!
//! Reads from the `uploads/` directory (same as `read_file`).
//! Output is capped at 1 000 rows; the column list and total row count are
//! always returned so the agent knows the full shape even when truncated.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MAX_ROWS: usize = 1_000;

#[derive(Deserialize)]
struct ReadCsvArgs {
    /// Filename inside the `uploads/` directory (e.g. `"sales_data.csv"`).
    filename: String,
    /// Field delimiter. Defaults to `,`; use `"\t"` for TSV files.
    #[serde(default = "default_delimiter")]
    delimiter: String,
}

fn default_delimiter() -> String { ",".to_string() }

#[derive(Serialize)]
struct CsvResult {
    filename:   String,
    columns:    Vec<String>,
    total_rows: usize,
    rows:       Vec<HashMap<String, serde_json::Value>>,
    truncated:  bool,
}

pub fn execute_read_csv(arguments: &str) -> Result<String, String> {
    let args: ReadCsvArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("read_csv: invalid arguments: {e}"))?;

    let name = args.filename.trim();
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(format!("read_csv: invalid filename '{name}'"));
    }
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(ext.as_str(), "csv" | "tsv" | "txt") {
        return Err(format!("read_csv: expected .csv/.tsv file, got '.{ext}'"));
    }

    let path = std::path::Path::new("uploads").join(name);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read_csv: cannot read '{name}': {e}"))?;

    let delimiter = args.delimiter.chars().next().unwrap_or(',');
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter as u8)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());

    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| format!("read_csv: failed to read headers: {e}"))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();
    let mut total_rows = 0usize;
    let mut truncated  = false;

    for result in rdr.records() {
        let record = result.map_err(|e| format!("read_csv: row {total_rows}: {e}"))?;
        total_rows += 1;
        if rows.len() >= MAX_ROWS {
            truncated = true;
            // Keep counting total but stop collecting rows.
            continue;
        }
        let mut map = HashMap::new();
        for (i, header) in headers.iter().enumerate() {
            let val = record.get(i).unwrap_or("").trim().to_string();
            // Coerce numeric-looking strings to numbers for easier agent use.
            let json_val = if let Ok(n) = val.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(f) = val.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|| serde_json::Value::String(val.clone()))
            } else if val.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(val)
            };
            map.insert(header.clone(), json_val);
        }
        rows.push(map);
    }

    let result = CsvResult {
        filename: name.to_string(),
        columns: headers,
        total_rows,
        rows,
        truncated,
    };
    serde_json::to_string(&result)
        .map_err(|e| format!("read_csv: serialisation failed: {e}"))
}
