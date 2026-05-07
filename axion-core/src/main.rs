mod planner;
mod dispatcher;
mod governor;
mod protocol;
mod engine;
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
    let f_id = plan.add_task("Research flights", vec![], AgentRole::WebSearcher);
    let h_id = plan.add_task("Find hotels", vec![f_id], AgentRole::WebSearcher);
    plan.add_task("Summarize trip", vec![h_id], AgentRole::Analyst);

    let mut attempts = 0;
    const MAX_ATTEMPTS: u8 = 10; // Higher limit for complex graphs

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
            // If nothing failed but we aren't done, it means we are waiting 
            // for dependencies to clear in the next tick.
            attempts += 1;
        } else {
            // HEALING: Governor marks failed tasks as Pending for the next tick
            governor::reset_failed_tasks(&mut plan.tasks);
            attempts += 1;
            println!("🚨 Failure detected. Attempting self-healing ({})...", attempts);
        }
    }
}