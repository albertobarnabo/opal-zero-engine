use axion_core::persistence::MissionSnapshot;
use axion_core::prelude::*;
use axion_kernel::prelude::{AxionGovernor, OpenAIProvider};
use axum::{
    extract::Path as AxumPath,
    http::{Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use tokio_stream::{wrappers::ReceiverStream, StreamExt as _};
use tower_http::cors::CorsLayer;

#[derive(Deserialize)]
struct TaskRequest {
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

async fn health() -> &'static str {
    "OK"
}

/// Convert a `MissionUpdate` into a typed SSE `Event`.
fn to_sse(update: MissionUpdate) -> Result<Event, Infallible> {
    let name = update.event_name();
    let data = serde_json::to_string(&update).unwrap_or_default();
    Ok(Event::default().event(name).data(data))
}

async fn execute(Json(req): Json<TaskRequest>) -> Response {
    let provider = match OpenAIProvider::new() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Provider init failed: {}", e) })),
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
        // Each request builds a fresh plan. run_mission clears context
        // defensively, so no state from a previous mission can leak.
        let mut plan = Plan::new(&intent);
        let f_id = plan.add_task(
            "The flight to Rome costs $300. Report this fact: 'Flight cost: $300'.",
            vec![],
            AgentRole::WebSearcher,
        );
        let h_id = plan.add_task(
            "The hotel in Rome costs $120 per night for 2 nights ($240 total). Report this fact: 'Hotel cost: $240'.",
            vec![f_id],
            AgentRole::WebSearcher,
        );
        let s_id = plan.add_task(
            "Use the calculator tool to add 300 + 240 and report the total trip cost.",
            vec![h_id],
            AgentRole::Analyst,
        );
        plan.add_task(
            "Save the trip report to 'trip_report.md' using the write_file tool. \
             The report must include: Flight cost: $300, Hotel cost: $240 (2 nights at $120), Total: $540.",
            vec![s_id],
            AgentRole::Analyst,
        );

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

async fn get_mission(AxumPath(id): AxumPath<String>) -> impl IntoResponse {
    if id.chars().any(|c| !c.is_alphanumeric() && c != '_') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid mission ID" })),
        )
            .into_response();
    }

    let path = std::path::Path::new("missions").join(format!("{}.json", id));
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(v) => (StatusCode::OK, Json(v)).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Corrupt snapshot" })),
            )
                .into_response(),
        },
        Err(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Mission not found" })),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    axion_core::registry::Registry::init_default();

    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let app = Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute))
        .route("/missions", get(list_missions))
        .route("/missions/:id", get(get_mission))
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to 0.0.0.0:8080");

    println!("🌐 Axion Server listening on http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}
