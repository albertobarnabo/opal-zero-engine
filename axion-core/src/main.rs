use axion_core::engine::OpenAIProvider;
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

    let provider = match OpenAIProvider::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("❌ Error: {}", e);
            std::process::exit(1);
        }
    };

    let mut plan = Plan::new(&args.intent);
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

    println!("🚀 Axion Core Heartbeat Started");

    match run_mission(&mut plan, &provider, 3, None).await {
        Ok(()) => println!("\n🎯 Mission Accomplished: Graph fully resolved."),
        Err(msg) => {
            eprintln!("❌ Mission failed: {}", msg);
            std::process::exit(1);
        }
    }
}
