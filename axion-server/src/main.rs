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
    routing::{get, post},
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

// ── UI presentation types (server layer only) ─────────────────────────────────

/// Glassmorphism / theme tokens for the web frontend.
///
/// These are presentation-layer concerns that live in the server, not the
/// kernel.  The Analyst may include them in the `finalize_mission_state` tool
/// call; the server extracts them here and forwards them to the client without
/// polluting `axion_core::protocol::MissionState`.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct DesignTokens {
    #[serde(default = "dt_primary_accent")]
    primary_accent: String,
    #[serde(default = "dt_glass_intensity")]
    glass_intensity: f32,
    #[serde(default = "dt_theme_preset")]
    theme_preset: String,
    #[serde(default = "dt_layout_density")]
    layout_density: String,
    #[serde(default = "dt_border_radius")]
    border_radius: u32,
    #[serde(default = "dt_surface_opacity")]
    surface_opacity: f32,
    #[serde(default = "dt_layout_strategy")]
    layout_strategy: String,
}

fn dt_primary_accent()   -> String { "#8b9cf4".to_string() }
fn dt_glass_intensity()  -> f32    { 0.60 }
fn dt_theme_preset()     -> String { "minimalist".to_string() }
fn dt_layout_density()   -> String { "spacious".to_string() }
fn dt_border_radius()    -> u32    { 24 }
fn dt_surface_opacity()  -> f32    { 0.06 }
fn dt_layout_strategy()  -> String { "Overview".to_string() }

impl Default for DesignTokens {
    fn default() -> Self {
        DesignTokens {
            primary_accent:  dt_primary_accent(),
            glass_intensity: dt_glass_intensity(),
            theme_preset:    dt_theme_preset(),
            layout_density:  dt_layout_density(),
            border_radius:   dt_border_radius(),
            surface_opacity: dt_surface_opacity(),
            layout_strategy: dt_layout_strategy(),
        }
    }
}

/// Wire representation of mission output sent to the frontend.
///
/// Wraps the kernel's `MissionState` (fact payload only) with UI-layer fields
/// so the frontend receives everything it needs in a single object.
/// Design tokens and widget hints are extracted from the raw task-result JSON
/// at the server boundary — they never enter the kernel's type system.
#[derive(Debug, Serialize, Deserialize, Clone)]
struct RichMissionState {
    intent_resolved: bool,
    data_payload: serde_json::Value,
    #[serde(default)]
    verification_logs: Vec<String>,
    #[serde(default)]
    design_tokens: DesignTokens,
    #[serde(default)]
    suggested_widgets: Vec<String>,
}

/// Build a `RichMissionState` from a raw JSON value that was produced by the
/// `finalize_mission_state` tool.  Design-token and widget-hint fields that
/// the Analyst embedded in the result JSON are promoted to the presentation
/// layer here; the kernel never sees them as typed fields.
fn enrich_mission_state(raw: serde_json::Value) -> RichMissionState {
    let intent_resolved = raw["intent_resolved"].as_bool().unwrap_or(true);
    let data_payload    = raw["data_payload"].clone();
    let verification_logs: Vec<String> = raw["verification_logs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let design_tokens: DesignTokens = raw
        .get("design_tokens")
        .and_then(|dt| serde_json::from_value(dt.clone()).ok())
        .unwrap_or_default();

    // Clamp visual ranges so invalid LLM output can't break the frontend.
    let design_tokens = DesignTokens {
        glass_intensity: design_tokens.glass_intensity.clamp(0.0, 1.0),
        surface_opacity: design_tokens.surface_opacity.clamp(0.0, 1.0),
        border_radius:   design_tokens.border_radius.min(48),
        primary_accent:  if design_tokens.primary_accent.starts_with('#') {
            design_tokens.primary_accent
        } else {
            dt_primary_accent()
        },
        ..design_tokens
    };

    let suggested_widgets: Vec<String> = raw["suggested_widgets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    RichMissionState {
        intent_resolved,
        data_payload,
        verification_logs,
        design_tokens,
        suggested_widgets,
    }
}

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
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    schema: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RefineRequest {
    intent: String,
    #[serde(default)]
    model: Option<String>,
}

/// Only allow a fixed set of known model IDs to prevent injection via this field.
fn validate_model(model: &str) -> bool {
    matches!(
        model,
        "gpt-4o-mini" | "gpt-4o" | "gpt-4o-mini-2024-07-18" | "o1-mini" | "o3-mini"
    )
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
///
/// For `MissionComplete` events, the raw `mission_state` JSON value is
/// enriched with UI-layer fields (design tokens, widget hints) extracted from
/// the Analyst's tool-call result before being forwarded to the client.
fn to_sse(update: MissionUpdate) -> Result<Event, Infallible> {
    let name = update.event_name();

    // For MissionComplete, replace the kernel's bare mission_state JSON with a
    // RichMissionState that includes design_tokens and suggested_widgets.
    let data = match update {
        MissionUpdate::MissionComplete {
            ref intent,
            task_count,
            expanded_task_count,
            ref mission_id,
            ref layout_hint,
            ref mission_state,
        } => {
            let rich_state = mission_state
                .as_ref()
                .map(|raw| enrich_mission_state(raw.clone()));

            serde_json::json!({
                "type": "mission_complete",
                "intent": intent,
                "task_count": task_count,
                "expanded_task_count": expanded_task_count,
                "mission_id": mission_id,
                "layout_hint": layout_hint,
                "mission_state": rich_state,
            })
            .to_string()
        }
        other => serde_json::to_string(&other).unwrap_or_default(),
    };

    Ok(Event::default().event(name).data(data))
}

async fn execute(headers: HeaderMap, Json(req): Json<TaskRequest>) -> Response {
    // Capture per-request API keys from custom headers.
    // These are stored in RequestKeys and threaded through the call chain, so
    // concurrent handlers never race on global environment variables.
    let openai_key  = headers.get("x-openai-key").and_then(|v| v.to_str().ok()).map(String::from);
    let tavily_key  = headers.get("x-tavily-key").and_then(|v| v.to_str().ok()).map(String::from);
    let av_key      = headers.get("x-alpha-vantage-key").and_then(|v| v.to_str().ok()).map(String::from);

    let request_keys = RequestKeys {
        openai_key:        openai_key.clone().filter(|k| !k.is_empty()),
        tavily_key:        tavily_key.filter(|k| !k.is_empty()),
        alpha_vantage_key: av_key.clone().filter(|k| !k.is_empty()),
    };

    // Model resolution: body → x-axion-model header → AXION_MODEL env → default
    let model = req.model.clone()
        .or_else(|| {
            headers
                .get("x-axion-model")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        });

    if let Some(ref m) = model {
        if !validate_model(m) {
            return (
                StatusCode::BAD_REQUEST,
                ApiError::new(
                    "UNSUPPORTED_MODEL",
                    "Unsupported model. Allowed: gpt-4o-mini, gpt-4o, o1-mini, o3-mini",
                ),
            )
                .into_response();
        }
    }

    let provider = match OpenAIProvider::new_with_explicit_key(
        model.clone(),
        openai_key.filter(|k| !k.is_empty()),
    ) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError::new("PROVIDER_ERROR", format!("Provider init failed: {}", e)),
            )
                .into_response();
        }
    };

    let governor = AxionGovernor::with_model(model.as_deref().unwrap_or("gpt-4o-mini"));

    // Channel capacity 64 — enough for a full mission with expansions without
    // back-pressure. Sends are fire-and-forget (errors silently dropped).
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(64);

    let intent = req.intent.clone();
    let schema = req.schema.clone();

    // ── Pre-flight: check if the schema requires Alpha Vantage and the key is absent ──
    // Financial schemas include keys like current_price_usd, price_history, etc.
    // If the key is missing we fail fast with a clear message instead of silently
    // producing empty cards after a full (expensive) mission run.
    const AV_KEYS: &[&str] = &[
        "current_price_usd", "price_history", "market_cap_usd", "pe_ratio",
        "eps", "revenue_ttm_usd", "income_history", "top_competitors",
        "analyst_consensus", "week_52_high_usd",
    ];
    let needs_av = schema.as_ref().map(|s| {
        s.as_object().map(|obj| {
            AV_KEYS.iter().any(|k| obj.contains_key(*k))
        }).unwrap_or(false)
    }).unwrap_or(false);

    let av_key_set = request_keys.alpha_vantage().is_some();

    if needs_av && !av_key_set {
        let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(1);
        let _ = tx.send(MissionUpdate::MissionFailed {
            error: "Alpha Vantage API key required for financial data (price history, \
                    fundamentals, competitors). Add your key in Settings or set \
                    ALPHA_VANTAGE_API_KEY on the server.".into(),
        }).await;
        drop(tx);
        let stream = ReceiverStream::new(rx).map(to_sse);
        return Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    }

    tokio::spawn(async move {
        // Build a dynamic plan from the user's intent via the LLM planner.
        // run_mission clears context defensively, so no state from a previous
        // mission can leak.
        let mut plan = build_plan_from_intent(&intent, &provider).await;

        // Inject the per-request API keys so tools can use them without
        // mutating global environment variables.
        plan.keys = request_keys;

        // If the caller supplied an output schema, inject it into the context bus
        // so the Analyst can include it in finalize_mission_state.
        if let Some(ref s) = schema {
            if let Ok(schema_str) = serde_json::to_string(s) {
                plan.context.data.insert("__output_schema".into(), schema_str);
            }
        }

        // tx is dropped when run_mission returns, which closes the SSE stream.
        let result = run_mission(&mut plan, &provider, &governor, 3, Some(tx)).await;
        if let Err(ref e) = result {
            if e.contains("time limit") {
                eprintln!("[timeout] mission hit wall-clock limit");
            }
        }
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
    // Capture per-request API keys — same pattern as the execute handler.
    let openai_key = headers.get("x-openai-key").and_then(|v| v.to_str().ok()).map(String::from);
    let request_keys = RequestKeys {
        openai_key:        openai_key.clone().filter(|k| !k.is_empty()),
        tavily_key:        headers.get("x-tavily-key").and_then(|v| v.to_str().ok()).map(String::from).filter(|k| !k.is_empty()),
        alpha_vantage_key: headers.get("x-alpha-vantage-key").and_then(|v| v.to_str().ok()).map(String::from).filter(|k| !k.is_empty()),
    };

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

    // Model resolution: body → x-axion-model header → AXION_MODEL env → default
    let model = req.model.clone()
        .or_else(|| {
            headers
                .get("x-axion-model")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        });

    if let Some(ref m) = model {
        if !validate_model(m) {
            in_flight.lock().await.remove(&id);
            return (
                StatusCode::BAD_REQUEST,
                ApiError::new(
                    "UNSUPPORTED_MODEL",
                    "Unsupported model. Allowed: gpt-4o-mini, gpt-4o, o1-mini, o3-mini",
                ),
            )
                .into_response();
        }
    }

    let provider = match OpenAIProvider::new_with_explicit_key(
        model.clone(),
        openai_key.filter(|k| !k.is_empty()),
    ) {
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

    let governor = AxionGovernor::with_model(model.as_deref().unwrap_or("gpt-4o-mini"));
    let (tx, rx) = tokio::sync::mpsc::channel::<MissionUpdate>(64);
    let refinement_intent = req.intent.clone();

    // Clone the Arc so the spawned task owns a reference to the in-flight set.
    let in_flight_cleanup = Arc::clone(&in_flight);
    let id_cleanup = id.clone();

    tokio::spawn(async move {
        let result = axion_core::refine_mission_with_keys(
            &snapshot,
            &refinement_intent,
            &provider,
            &governor,
            3,
            Some(tx),
            request_keys,
        )
        .await;

        if let Err(ref e) = result {
            if e.contains("time limit") {
                eprintln!("[timeout] mission {} hit wall-clock limit", id_cleanup);
            }
        }

        // Remove the mission ID from the in-flight set after streaming ends
        // (regardless of success or error).
        in_flight_cleanup.lock().await.remove(&id_cleanup);
    });

    let stream = ReceiverStream::new(rx).map(to_sse);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ── Clarify types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClarifyRequest {
    intent: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ClarifyQuestion {
    key:         String,
    question:    String,
    required:    bool,
    placeholder: String,
}

#[derive(Serialize, Deserialize)]
struct ClarifyResponse {
    complete:  bool,
    questions: Vec<ClarifyQuestion>,
}

/// `POST /api/v1/clarify`
///
/// Stateless pre-flight call. Sends the user intent to the LLM and returns 0–3
/// clarifying questions. Always returns HTTP 200 JSON. On any failure (provider
/// unavailable, parse error) returns `{ "complete": true, "questions": [] }` so
/// the caller can proceed to /execute without blocking the user.
async fn clarify_handler(
    headers: HeaderMap,
    Json(req): Json<ClarifyRequest>,
) -> impl IntoResponse {
    let fail_open = Json(ClarifyResponse { complete: true, questions: vec![] });

    if req.intent.trim().is_empty() {
        return fail_open;
    }

    // Capture per-request OpenAI key — pass directly to the provider instead of
    // calling set_var to avoid the concurrent-handler race condition.
    let openai_key = headers.get("x-openai-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .filter(|k| !k.is_empty());

    // Always use gpt-4o-mini — this call is cheap and speed matters.
    let provider = match OpenAIProvider::new_with_explicit_key(
        Some("gpt-4o-mini".to_string()),
        openai_key,
    ) {
        Ok(p)  => p,
        Err(_) => return fail_open,
    };

    let prompt = format!(
        "You are an intent completeness analyzer for an AI research assistant.\n\
Given the user's intent below, identify the MOST CRITICAL missing parameters — information that, \
if unknown, would cause the AI to guess and likely produce wrong or irrelevant output.\n\
\n\
Rules:\n\
- Return at most 3 questions. Fewer is better. If in doubt, return 0.\n\
- If the intent is self-contained and actionable as-is, return an empty questions array.\n\
- Do not ask about things the AI can reasonably assume or research itself.\n\
- Do ask about things that are personal, preference-based, or physically specific \
  (e.g. departure city, budget, date range, specific product version, target company name).\n\
- IMPORTANT: every question must have required = false. The user may not know the answer, \
  and the AI can still proceed with partial or no context. Never block the user.\n\
- Use a friendly, conversational tone. The placeholder should be a realistic example answer.\n\
\n\
Respond ONLY with valid JSON in this exact format:\n\
{{\"complete\": true | false, \"questions\": [\
{{\"key\": \"...\", \"question\": \"...\", \"required\": false, \"placeholder\": \"...\"}}]}}\n\
\n\
User intent: {}",
        req.intent
    );

    let text = match provider.generate_response(&prompt, None).await {
        Ok(axion_core::engine::ToolResponse::Text(t)) => t,
        _ => return fail_open,
    };

    // Strip optional markdown code fences before parsing.
    let json_str = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<ClarifyResponse>(json_str) {
        Ok(r)  => Json(r),
        Err(_) => fail_open,
    }
}

/// `GET /api/v1/config/status`
///
/// Reports which API keys are currently configured via environment variables.
/// Returns HTTP 200 JSON `{ "openai": bool, "tavily": bool, "alpha_vantage": bool }` — always succeeds.
async fn config_status_handler() -> impl IntoResponse {
    let openai = std::env::var("OPENAI_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let tavily = std::env::var("TAVILY_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let alpha_vantage = std::env::var("ALPHA_VANTAGE_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    Json(json!({ "openai": openai, "tavily": tavily, "alpha_vantage": alpha_vantage }))
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
            axum::http::header::HeaderName::from_static("x-alpha-vantage-key"),
            axum::http::header::HeaderName::from_static("x-axion-model"),
        ]);

    // Shared set tracking mission IDs that currently have an open refinement
    // SSE stream.  Prevents concurrent refinements for the same mission.
    let in_flight: InFlight = Arc::new(Mutex::new(HashSet::new()));

    // All routes except /health are versioned under /api/v1 and protected by
    // the auth middleware (auth is disabled when AXION_API_KEY is not set in env).
    let api_routes = Router::new()
        .route("/execute", post(execute))
        .route("/clarify", post(clarify_handler))
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

    println!(
        "🚀 Axion Server v{} starting — max_tokens={}, temperature={}, AV-key={}",
        env!("CARGO_PKG_VERSION"),
        std::env::var("AXION_MAX_TOKENS").unwrap_or_else(|_| "4096".into()),
        std::env::var("AXION_TEMPERATURE").unwrap_or_else(|_| "0.1".into()),
        if std::env::var("ALPHA_VANTAGE_API_KEY").is_ok() { "set ✓" } else { "not set" },
    );

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind to {}", addr));

    println!("🌐 Axion Server listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}
