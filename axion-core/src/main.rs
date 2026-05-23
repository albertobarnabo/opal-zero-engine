use axion_core::engine::SimpleProvider;
use axion_core::governor::BuiltinGovernor;
use axion_core::planner::Plan;
use axion_core::protocol::AgentRole;
use axion_core::run_mission;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    intent: String,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    axion_core::registry::Registry::init_default();

    let args = Args::parse();

    let provider = match SimpleProvider::openai("gpt-4o-mini") {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to create provider");
            std::process::exit(1);
        }
    };

    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new(&args.intent);
    // Each add_task call returns the task's slug, which downstream tasks
    // reference in their depends_on list.
    let flight_slug = plan.add_task(
        "The flight to Rome costs $300. Report this fact: 'Flight cost: $300'.",
        vec![],
        AgentRole::WebSearcher,
    );
    let hotel_slug = plan.add_task(
        "The hotel in Rome costs $120 per night for 2 nights ($240 total). Report this fact: 'Hotel cost: $240'.",
        vec![flight_slug],
        AgentRole::WebSearcher,
    );
    let calc_slug = plan.add_task(
        "Use the calculator tool to add 300 + 240 and report the total trip cost.",
        vec![hotel_slug],
        AgentRole::Analyst,
    );
    plan.add_task(
        "Save the trip report to 'trip_report.md' using the write_file tool. \
         The report must include: Flight cost: $300, Hotel cost: $240 (2 nights at $120), Total: $540.",
        vec![calc_slug],
        AgentRole::Analyst,
    );

    tracing::info!("Axion Core Heartbeat Started");

    match run_mission(&mut plan, &provider, &governor, 3, None).await {
        Ok(None) => tracing::info!("mission accomplished: graph fully resolved"),
        Ok(Some(hs)) => {
            tracing::info!(question = %hs.question, "mission paused — awaiting human feedback");
        }
        Err(msg) => {
            tracing::error!(error = %msg, "mission failed");
            std::process::exit(1);
        }
    }
}
