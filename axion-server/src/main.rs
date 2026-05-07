use axion_core::persistence::MissionSnapshot;
use axion_core::prelude::*;
use axum::{
    extract::Path as AxumPath,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
}

async fn health() -> &'static str {
    "OK"
}

async fn execute(Json(req): Json<TaskRequest>) -> impl IntoResponse {
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

    // Each request gets a completely fresh Plan (and therefore a fresh
    // ContextBus). run_mission also calls context.clear() defensively, so
    // no state from a previous mission can bleed into this one.
    let mut plan = Plan::new(&req.intent);
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

    let original_task_count = plan.tasks.len();

    match run_mission(&mut plan, &provider, 3).await {
        Ok(()) => {
            let expanded_task_count = plan.tasks.len().saturating_sub(original_task_count);
            (
                StatusCode::OK,
                Json(json!({
                    "status": "completed",
                    "intent": plan.original_intent,
                    "task_count": plan.tasks.len(),
                    "expanded_task_count": expanded_task_count,
                    "context": plan.context,
                })),
            )
                .into_response()
        }
        Err(msg) => {
            let expanded_task_count = plan.tasks.len().saturating_sub(original_task_count);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "status": "failed",
                    "error": msg,
                    "intent": plan.original_intent,
                    "task_count": plan.tasks.len(),
                    "expanded_task_count": expanded_task_count,
                    "context": plan.context,
                })),
            )
                .into_response()
        }
    }
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
                    });
                }
            }
        }
    }

    summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    (StatusCode::OK, Json(summaries)).into_response()
}

async fn get_mission(AxumPath(id): AxumPath<String>) -> impl IntoResponse {
    // Reject IDs that could be used for path traversal.
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
