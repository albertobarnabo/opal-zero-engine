use axion_core::engine::SimpleProvider;
use axion_core::governor::BuiltinGovernor;
use axion_core::mcp::AxionMcpServer;
use axion_core::planner::Plan;
use axion_core::protocol::AgentRole;
use axion_core::run_mission;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about = "Axion — autonomous AI agent kernel")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Run a one-shot mission with this intent (legacy positional mode).
    #[arg(short, long)]
    intent: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the Axion MCP server on stdin/stdout.
    ///
    /// Register it in Claude Code with:
    ///   claude mcp add axion -- axion mcp serve
    ///
    /// After registration, every Claude Code session in the project
    /// automatically has access to all Axion tools (web_search, calculator,
    /// sqlite_query, python_interpreter, …).
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Subcommand, Debug)]
enum McpAction {
    /// Run the MCP stdio server (used by MCP clients to discover and call tools).
    Serve,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    axion_core::registry::Registry::init_default();

    let args = Args::parse();

    // ── MCP server mode ───────────────────────────────────────────────────────
    if let Some(Command::Mcp { action: McpAction::Serve }) = args.command {
        if let Err(e) = AxionMcpServer::new().serve().await {
            eprintln!("Axion MCP server error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // ── Legacy one-shot mission mode ──────────────────────────────────────────
    let _intent = match args.intent {
        Some(ref i) => i.clone(),
        None => {
            eprintln!("Usage: axion --intent \"<task>\"  OR  axion mcp serve");
            std::process::exit(1);
        }
    };

    let provider = match SimpleProvider::openai("gpt-4o-mini") {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to create provider");
            std::process::exit(1);
        }
    };

    let governor = BuiltinGovernor::new();

    let mut plan = Plan::new(&_intent);
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
