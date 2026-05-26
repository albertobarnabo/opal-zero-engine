//! `sqlite_query` tool — run SQL against an embedded SQLite database.
//!
//! Agents can CREATE TABLE, INSERT rows, and SELECT results — all within a
//! lightweight SQLite file that persists for the duration of a mission.
//! The database file lives at `output/<mission_id>.db`; it is created
//! automatically on first use and removed when the mission completes.
//!
//! Using a per-mission file (rather than in-memory) means multiple tasks in
//! the same mission can share state: one task inserts rows, another queries
//! them.  The file is never shared across missions.
//!
//! Row output is capped at 500 rows and serialised as a JSON array of objects.

use rusqlite::{Connection, params_from_iter, types::ValueRef};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const MAX_ROWS: usize = 500;

#[derive(Deserialize)]
struct SqliteArgs {
    /// The SQL statement to execute (SELECT, CREATE TABLE, INSERT, UPDATE, …).
    query: String,
    /// Optional list of positional parameters bound to `?` placeholders.
    #[serde(default)]
    params: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct SqliteResult {
    rows: Vec<HashMap<String, serde_json::Value>>,
    rows_affected: usize,
    truncated: bool,
}

pub fn execute_sqlite_query(arguments: &str, mission_id: &str) -> Result<String, String> {
    let args: SqliteArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("sqlite_query: invalid arguments: {e}"))?;

    let db_path = db_path_for_mission(mission_id);
    let conn = open_connection(&db_path)?;

    // Determine whether this is a query (returns rows) or a statement (no rows).
    let trimmed = args.query.trim().to_uppercase();
    let is_query = trimmed.starts_with("SELECT")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("PRAGMA")
        || trimmed.starts_with("EXPLAIN");

    if is_query {
        execute_select(&conn, &args.query, &args.params)
    } else {
        execute_write(&conn, &args.query, &args.params)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn db_path_for_mission(mission_id: &str) -> PathBuf {
    // Sanitise mission_id — keep only alphanumeric chars and underscores.
    let safe_id: String = mission_id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect();

    std::fs::create_dir_all("output").ok();
    PathBuf::from(format!("output/{safe_id}.db"))
}

fn open_connection(path: &PathBuf) -> Result<Connection, String> {
    Connection::open(path)
        .map_err(|e| format!("sqlite_query: failed to open database: {e}"))
}

/// Convert bound params from `serde_json::Value` to rusqlite params.
macro_rules! bind_params {
    ($params:expr) => {{
        $params
            .iter()
            .map(|v| match v {
                serde_json::Value::Null    => rusqlite::types::Value::Null,
                serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(*b as i64),
                serde_json::Value::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        rusqlite::types::Value::Integer(i)
                    } else {
                        rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0))
                    }
                }
                serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                other => rusqlite::types::Value::Text(other.to_string()),
            })
            .collect::<Vec<_>>()
    }};
}

fn execute_select(
    conn: &Connection,
    query: &str,
    params: &[serde_json::Value],
) -> Result<String, String> {
    let bound = bind_params!(params);
    let mut stmt = conn
        .prepare(query)
        .map_err(|e| format!("sqlite_query: prepare failed: {e}"))?;

    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let rows_iter = stmt
        .query(params_from_iter(bound.iter()))
        .map_err(|e| format!("sqlite_query: query failed: {e}"))?;

    let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();
    let mut truncated = false;

    let mut rows_iter = rows_iter;
    while let Some(row) = rows_iter.next().map_err(|e| format!("sqlite_query: row error: {e}"))? {
        if rows.len() >= MAX_ROWS {
            truncated = true;
            break;
        }
        let mut map = HashMap::new();
        for (i, col) in col_names.iter().enumerate() {
            let val = match row.get_ref(i).map_err(|e| format!("sqlite_query: column {i}: {e}"))? {
                ValueRef::Null         => serde_json::Value::Null,
                ValueRef::Integer(n)   => serde_json::Value::Number(n.into()),
                ValueRef::Real(f)      => {
                    serde_json::Number::from_f64(f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)
                }
                ValueRef::Text(b)  => {
                    serde_json::Value::String(String::from_utf8_lossy(b).into_owned())
                }
                ValueRef::Blob(b) => {
                    serde_json::Value::String(format!("<blob {} bytes>", b.len()))
                }
            };
            map.insert(col.clone(), val);
        }
        rows.push(map);
    }

    let result = SqliteResult { rows, rows_affected: 0, truncated };
    serde_json::to_string(&result)
        .map_err(|e| format!("sqlite_query: serialisation failed: {e}"))
}

fn execute_write(
    conn: &Connection,
    query: &str,
    params: &[serde_json::Value],
) -> Result<String, String> {
    let bound = bind_params!(params);
    conn.execute(query, params_from_iter(bound.iter()))
        .map_err(|e| format!("sqlite_query: execute failed: {e}"))?;

    let rows_affected = conn.changes() as usize;
    let result = SqliteResult { rows: vec![], rows_affected, truncated: false };
    serde_json::to_string(&result)
        .map_err(|e| format!("sqlite_query: serialisation failed: {e}"))
}
