use axion_core::persistence::MissionSnapshot;
use axion_core::prelude::*;
use axion_kernel::prelude::{AxionGovernor, OpenAIProvider};
use axum::{
    extract::{Multipart, Path as AxumPath, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_stream::{wrappers::ReceiverStream, StreamExt as _};
use tower_http::cors::CorsLayer;

// ── Standard API error type ───────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct ApiError {
    error: String,
    code:  String,
}

impl ApiError {
    fn new(code: &str, msg: impl Into<String>) -> axum::Json<Self> {
        axum::Json(Self { error: msg.into(), code: code.into() })
    }
}

/// Shared set of mission IDs currently undergoing a refinement SSE stream.
/// Used to return HTTP 409 when a concurrent refinement is attempted.
type InFlight = Arc<Mutex<HashSet<String>>>;

#[derive(Deserialize)]
struct TaskRequest {
    intent: String,
}

#[derive(Deserialize)]
struct RefineRequest {
    intent: String,
}

#[derive(Serialize)]
struct MissionSummary {
    id: String,
    timestamp: u64,
    intent: String,
    task_count: usize,
    status: String,
    #[serde(default)]
    layout_hint: String,
}

// ── Auth middleware ───────────────────────────────────────────────────────────

async fn auth_middleware(
    headers: axum::http::HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let required_key = std::env::var("AXION_API_KEY").unwrap_or_default();

    // If no key is configured, auth is disabled (local dev mode).
    if required_key.is_empty() {
        return next.run(request).await;
    }

    let provided = headers
        .get("x-axion-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided != required_key {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            ApiError::new("UNAUTHORIZED", "Missing or invalid X-Axion-Key header"),
        )
            .into_response();
    }

    next.run(request).await
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn health() -> &'static str {
    "OK"
}

/// Convert a `MissionUpdate` into a typed SSE `Event`.
fn to_sse(update: MissionUpdate) -> Result<Event, Infallible> {
    let name = update.event_name();
    let data = serde_json::to_string(&update).unwrap_or_default();
    Ok(Event::default().event(name).data(data))
}

async fn execute(headers: HeaderMap, Json(req): Json<TaskRequest>) -> Response {
    // Allow the frontend to supply API keys per-request via custom headers.
    // Safety: this is a local single-user tool with no concurrent env readers
    // outside Rust code, so the relaxed atomicity of set_var is acceptable.
    if let Some(key) = headers.get("x-openai-key").and_then(|v| v.to_str().ok()) {
        if !key.is_empty() {
            // SAFETY: single-user local tool; no signal handlers read env vars.
            unsafe { std::env::set_var("OPENAI_API_KEY", key) };
        }
    }
    if let Some(key) = headers.get("x-tavily-key").and_then(|v| v.to_str().ok()) {
        if !key.is_empty() {
            // SAFETY: same rationale as above.
            unsafe { std::env::set_var("TAVILY_API_KEY", key) };
        }
    }

    let provider = match OpenAIProvider::new() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("PROVIDER_ERROR", format!("Provider init failed: {}", e)),
            )
                .into_response();
        }
    };

    let governor = AxionGovernor::new();

    // Channel capacity 64 — enough for a full mission with expansions without
    // back-pressure. Sends are fire-and-forget (errors silently dropped).
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(64);

    let intent = req.intent.clone();
    tokio::spawn(async move {
        // Build a dynamic plan from the user's intent via the LLM planner.
        // run_mission clears context defensively, so no state from a previous
        // mission can leak.
        let mut plan = build_plan_from_intent(&intent, &provider).await;

        // tx is dropped when run_mission returns, which closes the SSE stream.
        let _ = run_mission(&mut plan, &provider, &governor, 3, Some(tx)).await;
    });

    let stream = ReceiverStream::new(rx).map(to_sse);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn list_missions() -> impl IntoResponse {
    let missions_dir = std::path::Path::new("missions");
    let mut summaries: Vec<MissionSummary> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(missions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(snap) = serde_json::from_str::<MissionSnapshot>(&content) {
                    summaries.push(MissionSummary {
                        id: snap.id,
                        timestamp: snap.timestamp,
                        intent: snap.intent,
                        task_count: snap.task_count,
                        status: snap.status,
                        layout_hint: snap.layout_hint,
                    });
                }
            }
        }
    }

    summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    (StatusCode::OK, Json(summaries)).into_response()
}

async fn delete_mission(AxumPath(id): AxumPath<String>) -> impl IntoResponse {
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_MISSION_ID", "Invalid mission ID"),
        )
            .into_response();
    }
    let path = std::path::Path::new("missions").join(format!("{}.json", id));
    match std::fs::remove_file(&path) {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            ApiError::new("MISSION_NOT_FOUND", "Mission not found"),
        )
            .into_response(),
    }
}

/// `GET /api/v1/missions/:id/export?format=md|csv|html`
///
/// Reads the saved mission snapshot, derives a human-readable document from
/// the `mission_state.data_payload`, and streams it back as a file download.
async fn export_mission(
    AxumPath(id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_MISSION_ID", "Invalid mission ID"),
        )
            .into_response();
    }

    let format = params
        .get("format")
        .map(|f| f.to_lowercase())
        .unwrap_or_else(|| "md".to_string());

    if !matches!(format.as_str(), "md" | "csv" | "html") {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_FORMAT", "format must be md, csv, or html"),
        )
            .into_response();
    }

    let path = std::path::Path::new("missions").join(format!("{}.json", id));
    let content_str = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                ApiError::new("MISSION_NOT_FOUND", "Mission not found"),
            )
                .into_response();
        }
    };

    let snap: serde_json::Value = match serde_json::from_str(&content_str) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("CORRUPT_SNAPSHOT", "Corrupt snapshot"),
            )
                .into_response();
        }
    };

    let intent = snap["intent"].as_str().unwrap_or("Mission Report").to_string();
    let payload = snap["mission_state"]["data_payload"]
        .as_object()
        .cloned()
        .unwrap_or_default();

    let document = match format.as_str() {
        "md" => render_md(&intent, &payload),
        "csv" => render_csv(&intent, &payload),
        "html" => render_html(&intent, &payload),
        _ => unreachable!(),
    };

    let (content_type, ext) = match format.as_str() {
        "csv"  => ("text/csv; charset=utf-8", "csv"),
        "html" => ("text/html; charset=utf-8", "html"),
        _      => ("text/markdown; charset=utf-8", "md"),
    };

    let filename = format!("axion-report-{}.{}", id, ext);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_DISPOSITION,
                Box::leak(
                    format!("attachment; filename=\"{}\"", filename).into_boxed_str()
                ) as &str,
            ),
        ],
        document,
    )
        .into_response()
}

fn render_md(intent: &str, payload: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut out = format!("# Mission Report: {}\n\n", intent);
    for (key, value) in payload {
        let title = key_to_title(key);
        out += &format!("## {}\n\n", title);
        render_value_md(value, &mut out);
        out += "\n";
    }
    out
}

fn render_csv(intent: &str, payload: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut out = format!("Mission Report: {}\n\n", intent);
    for (key, value) in payload {
        let title = key_to_title(key);
        out += &format!("## {}\n", title);
        if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first().and_then(|v| v.as_object()) {
                let headers: Vec<&str> = first.keys().map(|k| k.as_str()).collect();
                out += &headers.iter().map(|h| csv_esc(h)).collect::<Vec<_>>().join(",");
                out += "\n";
                for item in arr {
                    if let Some(obj) = item.as_object() {
                        let row: Vec<String> = headers
                            .iter()
                            .map(|h| csv_esc(&val_str(obj.get(*h).unwrap_or(&serde_json::Value::Null))))
                            .collect();
                        out += &row.join(",");
                        out += "\n";
                    }
                }
            } else {
                for item in arr {
                    out += &csv_esc(&val_str(item));
                    out += "\n";
                }
            }
        } else {
            out += &csv_esc(&val_str(value));
            out += "\n";
        }
        out += "\n";
    }
    out
}

fn render_html(intent: &str, payload: &serde_json::Map<String, serde_json::Value>) -> String {
    let mut body = String::new();
    for (key, value) in payload {
        let title = key_to_title(key);
        body += &format!("<section><h2>{}</h2>\n", esc_html(&title));
        if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first().and_then(|v| v.as_object()) {
                let headers: Vec<&str> = first.keys().map(|k| k.as_str()).collect();
                body += "<table><thead><tr>";
                for h in &headers { body += &format!("<th>{}</th>", esc_html(h)); }
                body += "</tr></thead><tbody>\n";
                for item in arr {
                    body += "<tr>";
                    if let Some(obj) = item.as_object() {
                        for h in &headers {
                            body += &format!("<td>{}</td>", esc_html(&val_str(obj.get(*h).unwrap_or(&serde_json::Value::Null))));
                        }
                    }
                    body += "</tr>\n";
                }
                body += "</tbody></table>";
            } else {
                body += "<ul>";
                for item in arr { body += &format!("<li>{}</li>", esc_html(&val_str(item))); }
                body += "</ul>";
            }
        } else {
            body += &format!("<p>{}</p>", esc_html(&val_str(value)));
        }
        body += "</section>\n";
    }

    format!(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<title>{t}</title>
<style>body{{font-family:-apple-system,BlinkMacSystemFont,Inter,sans-serif;background:#07090c;color:#e5e7eb;max-width:900px;margin:40px auto;padding:0 24px}}
h1{{font-size:2rem;font-weight:900;color:#fff;margin-bottom:6px}}h2{{font-size:1.15rem;font-weight:700;margin-top:32px;color:rgba(255,255,255,.85)}}
p{{color:rgba(255,255,255,.6);line-height:1.7}}
table{{width:100%;border-collapse:collapse;margin-top:10px}}
th{{text-align:left;padding:8px 12px;background:rgba(255,255,255,.06);color:rgba(255,255,255,.8);font-size:11px;text-transform:uppercase;letter-spacing:.06em;border-bottom:1px solid rgba(255,255,255,.1)}}
td{{padding:8px 12px;color:rgba(255,255,255,.6);border-bottom:1px solid rgba(255,255,255,.05);font-size:14px}}
ul{{padding-left:20px}}li{{color:rgba(255,255,255,.6);line-height:1.7;font-size:14px}}</style>
</head><body><h1>{t}</h1><p style="font-size:12px;color:rgba(255,255,255,.28)">Axion Intelligence Report</p>
{body}</body></html>"#,
        t = esc_html(intent),
        body = body,
    )
}

fn render_value_md(value: &serde_json::Value, out: &mut String) {
    if let Some(arr) = value.as_array() {
        if let Some(first) = arr.first().and_then(|v| v.as_object()) {
            let headers: Vec<&str> = first.keys().map(|k| k.as_str()).collect();
            *out += &format!("| {} |\n", headers.join(" | "));
            *out += &format!("| {} |\n", headers.iter().map(|_| "---").collect::<Vec<_>>().join(" | "));
            for item in arr {
                if let Some(obj) = item.as_object() {
                    let row: Vec<String> = headers.iter()
                        .map(|h| val_str(obj.get(*h).unwrap_or(&serde_json::Value::Null)))
                        .collect();
                    *out += &format!("| {} |\n", row.join(" | "));
                }
            }
        } else {
            for item in arr { *out += &format!("- {}\n", val_str(item)); }
        }
    } else if let Some(obj) = value.as_object() {
        for (k, v) in obj {
            *out += &format!("**{}**: {}\n\n", key_to_title(k), val_str(v));
        }
    } else {
        *out += &val_str(value);
        *out += "\n";
    }
}

fn key_to_title(key: &str) -> String {
    key.replace('_', " ")
       .split_whitespace()
       .map(|w| {
           let mut chars = w.chars();
           match chars.next() {
               None => String::new(),
               Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
           }
       })
       .collect::<Vec<_>>()
       .join(" ")
}

fn val_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null      => String::new(),
        other                        => other.to_string(),
    }
}

fn csv_esc(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

async fn get_mission(AxumPath(id): AxumPath<String>) -> impl IntoResponse {
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_MISSION_ID", "Invalid mission ID"),
        )
            .into_response();
    }

    let path = std::path::Path::new("missions").join(format!("{}.json", id));
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(v) => (StatusCode::OK, Json(v)).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("CORRUPT_SNAPSHOT", "Corrupt snapshot"),
            )
                .into_response(),
        },
        Err(_) => (
            StatusCode::NOT_FOUND,
            ApiError::new("MISSION_NOT_FOUND", "Mission not found"),
        )
            .into_response(),
    }
}

/// `POST /api/v1/missions/:id/refine`
///
/// Loads an existing mission snapshot and runs a targeted refinement pass
/// against it, streaming `MissionUpdate` SSE events back to the caller.
/// On completion the original `missions/<id>.json` is overwritten in-place.
///
/// Returns HTTP 409 Conflict immediately if a refinement stream is already
/// open for the same mission ID (concurrent refinement guard).
async fn refine_mission_handler(
    State(in_flight): State<InFlight>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<RefineRequest>,
) -> Response {
    // Honour per-request API key headers (same logic as the execute handler).
    if let Some(key) = headers.get("x-openai-key").and_then(|v| v.to_str().ok()) {
        if !key.is_empty() {
            // SAFETY: single-user local tool; no signal handlers read env vars.
            unsafe { std::env::set_var("OPENAI_API_KEY", key) };
        }
    }
    if let Some(key) = headers.get("x-tavily-key").and_then(|v| v.to_str().ok()) {
        if !key.is_empty() {
            // SAFETY: same rationale as above.
            unsafe { std::env::set_var("TAVILY_API_KEY", key) };
        }
    }
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_MISSION_ID", "Invalid mission ID"),
        )
            .into_response();
    }
    if req.intent.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            ApiError::new("INVALID_INTENT", "Refinement intent must not be empty"),
        )
            .into_response();
    }

    // ── Concurrent-refinement guard ──────────────────────────────────────────
    {
        let mut set = in_flight.lock().await;
        if set.contains(&id) {
            return (
                StatusCode::CONFLICT,
                ApiError::new("ALREADY_REFINING", "A refinement for this mission is already in progress"),
            )
                .into_response();
        }
        set.insert(id.clone());
    }

    let snapshot = match axion_core::persistence::load_snapshot(&id) {
        Ok(s)  => s,
        Err(e) => {
            // Release guard before returning early.
            in_flight.lock().await.remove(&id);
            return (
                StatusCode::NOT_FOUND,
                ApiError::new("MISSION_NOT_FOUND", e),
            )
                .into_response();
        }
    };

    let provider = match OpenAIProvider::new() {
        Ok(p)  => p,
        Err(e) => {
            in_flight.lock().await.remove(&id);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("PROVIDER_ERROR", format!("Provider init failed: {}", e)),
            )
                .into_response();
        }
    };

    let governor = AxionGovernor::new();
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(64);
    let refinement_intent = req.intent.clone();

    // Clone the Arc so the spawned task owns a reference to the in-flight set.
    let in_flight_cleanup = Arc::clone(&in_flight);
    let id_cleanup = id.clone();

    tokio::spawn(async move {
        let _ = axion_core::refine_mission(
            &snapshot,
            &refinement_intent,
            &provider,
            &governor,
            3,
            Some(tx),
        )
        .await;

        // Remove the mission ID from the in-flight set after streaming ends
        // (regardless of success or error).
        in_flight_cleanup.lock().await.remove(&id_cleanup);
    });

    let stream = ReceiverStream::new(rx).map(to_sse);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `GET /api/v1/config/status`
///
/// Reports which API keys are currently configured via environment variables.
/// Returns HTTP 200 JSON `{ "openai": bool, "tavily": bool }` — always succeeds.
async fn config_status_handler() -> impl IntoResponse {
    let openai = std::env::var("OPENAI_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let tavily = std::env::var("TAVILY_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    Json(json!({ "openai": openai, "tavily": tavily }))
}

/// `POST /api/v1/upload`
///
/// Accepts a `multipart/form-data` body with a single field named `"file"`.
/// Validates that the file is an image or data file, writes it to
/// `uploads/<uuid>.<ext>`, and returns `{ "filename", "file_type", "original_name" }`.
async fn upload_handler(mut multipart: Multipart) -> Response {
    while let Ok(Some(field)) = multipart.next_field().await {
        // Only process the named "file" field; skip everything else.
        if field.name().unwrap_or("") != "file" {
            continue;
        }

        // ── Content-Type + extension allow-list ──────────────────────────────
        let content_type = field.content_type().unwrap_or("").to_string();
        let original_name = field.file_name().unwrap_or("upload.bin").to_string();
        let ext = std::path::Path::new(&original_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Accepted: images + common data file types.
        let is_image = content_type.starts_with("image/");
        let is_data  = matches!(ext.as_str(), "csv" | "json" | "txt")
            || matches!(
                content_type.as_str(),
                "text/csv" | "application/json" | "text/plain"
            );

        if !is_image && !is_data {
            return (
                StatusCode::BAD_REQUEST,
                ApiError::new("UPLOAD_TYPE_REJECTED", "Only image, CSV, JSON, and TXT files are accepted"),
            )
                .into_response();
        }

        // Derive a safe extension: fall back to type-specific defaults.
        let ext = if ext.is_empty() {
            if is_image { "jpg" } else { "txt" }.to_string()
        } else {
            ext
        };

        let filename = format!("{}.{}", uuid::Uuid::new_v4(), ext);

        // ── Read bytes (consumes field) ────────────────────────────────────────
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiError::new("UPLOAD_READ_FAILED", format!("Failed to read upload: {}", e)),
                )
                    .into_response();
            }
        };

        // ── Write to uploads/ ─────────────────────────────────────────────────
        if let Err(e) = tokio::fs::create_dir_all("uploads").await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("UPLOAD_IO_ERROR", format!("Failed to create uploads directory: {}", e)),
            )
                .into_response();
        }

        if let Err(e) = tokio::fs::write(format!("uploads/{}", filename), &bytes).await {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("UPLOAD_IO_ERROR", format!("Failed to write file: {}", e)),
            )
                .into_response();
        }

        let file_type = if is_image { "image" } else { "data" };
        println!("📎 Upload saved: uploads/{} ({})", filename, file_type);
        return (StatusCode::OK, Json(json!({
            "filename": filename,
            "file_type": file_type,
            "original_name": original_name,
        }))).into_response();
    }

    (
        StatusCode::BAD_REQUEST,
        ApiError::new("UPLOAD_MISSING_FILE", "No field named 'file' found in the request"),
    )
        .into_response()
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    axion_core::registry::Registry::init_default();

    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::HeaderName::from_static("x-axion-key"),
            axum::http::header::HeaderName::from_static("x-openai-key"),
            axum::http::header::HeaderName::from_static("x-tavily-key"),
        ]);

    // Shared set tracking mission IDs that currently have an open refinement
    // SSE stream.  Prevents concurrent refinements for the same mission.
    let in_flight: InFlight = Arc::new(Mutex::new(HashSet::new()));

    // All routes except /health are versioned under /api/v1 and protected by
    // the auth middleware (auth is disabled when AXION_API_KEY is not set in env).
    let api_routes = Router::new()
        .route("/execute", post(execute))
        .route("/missions", get(list_missions))
        .route("/missions/:id", get(get_mission).delete(delete_mission))
        .route("/missions/:id/export", get(export_mission))
        .route("/missions/:id/refine", post(refine_mission_handler))
        .route("/upload", post(upload_handler))
        .route("/config/status", get(config_status_handler))
        .layer(axum::middleware::from_fn(auth_middleware))
        .with_state(in_flight);

    let app = Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api_routes)
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to 0.0.0.0:8080");

    println!("🌐 Axion Server listening on http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}
