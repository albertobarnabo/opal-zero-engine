use axion_core::prelude::*;
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct TaskRequest {
    intent: String,
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

    match run_mission(&mut plan, &provider, 3).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "completed",
                "context": plan.context,
            })),
        )
            .into_response(),
        Err(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "status": "failed",
                "error": msg,
                "context": plan.context,
            })),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let app = Router::new()
        .route("/health", get(health))
        .route("/execute", post(execute));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("Failed to bind to 0.0.0.0:8080");

    println!("🌐 Axion Server listening on http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}
