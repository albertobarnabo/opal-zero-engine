mod planner;
mod dispatcher;
mod governor;
mod protocol;
mod engine;
mod tools;
use engine::OpenAIProvider;
use clap::Parser;
use planner::Plan;
use protocol::AgentRole; 

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    intent: String,
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenvy::dotenv().ok();

    let args = Args::parse();

    // Initialize OpenAI provider with API key from environment
    let provider = match OpenAIProvider::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("❌ Error: {}", e);
            std::process::exit(1);
        }
    };

    // --- STAGE 1: PLANNING ---
    let mut plan = Plan::new(&args.intent);
    let f_id = plan.add_task("The flight to Rome costs $300. Report this fact: 'Flight cost: $300'.", vec![], AgentRole::WebSearcher);
    let h_id = plan.add_task("The hotel in Rome costs $120 per night for 2 nights ($240 total). Report this fact: 'Hotel cost: $240'.", vec![f_id], AgentRole::WebSearcher);
    plan.add_task("Use the calculator tool to add 300 + 240 and report the total trip cost.", vec![h_id], AgentRole::Analyst);

    let mut attempts = 0;
    const MAX_ATTEMPTS: u8 = 3; // Cap retries at 3

    println!("🚀 Axion Core Heartbeat Started");

    while attempts < MAX_ATTEMPTS {
        // --- STAGE 2: DISPATCHING ---
        // We pass the whole list. Dispatcher only runs what is 'Pending' & 'Ready'
        dispatcher::dispatch_tasks(&mut plan.tasks, &mut plan.context, &provider).await;

        // --- STAGE 3: GOVERNOR ---
        let report = governor::review_execution(&plan.tasks);

        if report.all_successful {
            println!("\n🎯 Mission Accomplished: Graph fully resolved.");
            return;
        }

        if report.failed_tasks.is_empty() {
            attempts += 1;
        } else {
            governor::reset_failed_tasks(&mut plan.tasks);
            attempts += 1;
            println!("🚨 Failure detected. Attempting self-healing ({}/{})...", attempts, MAX_ATTEMPTS);
        }
    }

    eprintln!("❌ Mission failed: {} task(s) could not be completed after {} attempts.",
        plan.tasks.iter().filter(|t| !matches!(t.status, crate::protocol::TaskStatus::Completed)).count(),
        MAX_ATTEMPTS
    );
    std::process::exit(1);
}