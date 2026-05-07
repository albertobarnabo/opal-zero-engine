pub async fn execute_tool(name: &str, arguments: &str) -> Result<String, String> {
    match name {
        "calculator" => execute_calculator(arguments),
        "web_search" => execute_web_search(arguments).await,
        "write_file" => execute_write_file(arguments),
        _ => Err(format!("Unknown tool: {}", name)),
    }
}

fn execute_calculator(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct CalcArgs {
        operation: String,
        #[serde(default)]
        values: Vec<f64>,
    }

    let args: CalcArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse calculator arguments: {}", e))?;

    if args.values.is_empty() {
        return Err("No values provided for calculation".to_string());
    }

    let result = match args.operation.as_str() {
        "add" => args.values.iter().sum::<f64>(),
        "subtract" => args.values[1..].iter().fold(args.values[0], |acc, &x| acc - x),
        "multiply" => args.values.iter().fold(1.0, |acc, &x| acc * x),
        "divide" => {
            let mut result = args.values[0];
            for &val in &args.values[1..] {
                if val == 0.0 {
                    return Err("Division by zero".to_string());
                }
                result /= val;
            }
            result
        }
        _ => return Err(format!("Unknown operation: {}", args.operation)),
    };

    Ok(format!("Result: {}", result))
}

fn execute_write_file(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct WriteArgs {
        filename: String,
        content: String,
    }

    let args: WriteArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse write_file arguments: {}", e))?;

    // Safety: reject filenames with path separators or traversal sequences.
    if args.filename.contains('/') || args.filename.contains('\\') || args.filename.contains("..") {
        return Err(format!(
            "Invalid filename '{}': must not contain path separators or '..'",
            args.filename
        ));
    }

    let output_dir = std::path::Path::new("output");
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output/ directory: {}", e))?;

    let path = output_dir.join(&args.filename);
    std::fs::write(&path, &args.content)
        .map_err(|e| format!("Failed to write '{}': {}", args.filename, e))?;

    Ok(format!("File 'output/{}' written successfully.", args.filename))
}

async fn execute_web_search(arguments: &str) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct SearchArgs {
        query: String,
    }

    let args: SearchArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("Failed to parse search arguments: {}", e))?;

    // Use the real Tavily API when a key is present; otherwise fall back to a
    // detailed simulation that gives the LLM enough material to reason with.
    match std::env::var("TAVILY_API_KEY") {
        Ok(key) => tavily_search(&args.query, &key).await,
        Err(_) => Ok(simulated_search(&args.query)),
    }
}

async fn tavily_search(query: &str, api_key: &str) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct TavilyRequest<'a> {
        api_key: &'a str,
        query: &'a str,
        search_depth: &'a str,
        max_results: u8,
    }

    #[derive(serde::Deserialize)]
    struct TavilyResponse {
        results: Vec<TavilyResult>,
    }

    #[derive(serde::Deserialize)]
    struct TavilyResult {
        title: String,
        url: String,
        content: String,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.tavily.com/search")
        .json(&TavilyRequest {
            api_key,
            query,
            search_depth: "basic",
            max_results: 5,
        })
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Tavily API error {}: {}", status, body));
    }

    let body: TavilyResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {}", e))?;

    if body.results.is_empty() {
        return Ok(format!("No results found for '{}'.", query));
    }

    let formatted = body
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}\n   {}\n   Source: {}", i + 1, r.title, r.content, r.url))
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(format!("Live search results for '{}':\n\n{}", query, formatted))
}

fn simulated_search(query: &str) -> String {
    let q = query.to_lowercase();

    if q.contains("flight") || (q.contains("rome") && (q.contains("fly") || q.contains("air"))) {
        return format!(
            "Search results for '{query}':\n\n\
            1. **Direct Flights to Rome (FCO) — Round Trip from JFK**\n   \
            - Delta Airlines: $289 (direct, 9h 15m) ⭐ Best value\n   \
            - Alitalia / ITA Airways: $312 (direct, 9h 30m)\n   \
            - Lufthansa: $268 (1 stop via Frankfurt, 11h 45m)\n\n\
            2. **Budget Options (indirect)**\n   \
            - Vueling via Barcelona: $198 (12h 30m total)\n   \
            - Ryanair via Madrid: $175 (14h total, carry-on fees apply)\n\n\
            3. **Booking Tips**\n   \
            - Book 6–8 weeks ahead for best prices.\n   \
            - Tuesday/Wednesday departures average 15% cheaper.\n   \
            - Baggage fees not included in budget airline fares.",
        );
    }

    if q.contains("hotel") || (q.contains("rome") && (q.contains("stay") || q.contains("accommodation"))) {
        return format!(
            "Search results for '{query}':\n\n\
            1. **4-Star Hotels — Rome City Centre**\n   \
            - Hotel Nazionale: $145/night (near Pantheon, breakfast included) ⭐ Best value\n   \
            - The Inn at the Spanish Steps: $210/night (boutique luxury)\n   \
            - Bettoja Hotels: $138/night (near Termini, reliable chain)\n\n\
            2. **Mid-Range / Budget**\n   \
            - Residenza Paolo VI: $120/night (Vatican views, 3-star)\n   \
            - Hotel Artorius: $89/night (Trastevere neighbourhood)\n   \
            - Generator Rome (Hostel): $52/night (private room)\n\n\
            3. **Booking Tips**\n   \
            - Most 3–4 star hotels include colazione (Italian breakfast).\n   \
            - Trastevere and Prati districts offer better value than the historic centre.\n   \
            - Book 3+ weeks ahead; prices rise sharply for popular dates.",
        );
    }

    format!(
        "Search results for '{query}':\n\n\
        1. **Destination Overview — Rome, Italy**\n   \
        Rome is a world-class destination with 2,500+ years of history. \
        The city hosts the Colosseum, Vatican Museums, Trevi Fountain, and Piazza Navona.\n\n\
        2. **Best Time to Visit**\n   \
        - Spring (Apr–May): Ideal weather, 18–22°C, moderate crowds.\n   \
        - Autumn (Sep–Oct): Warm, fewer tourists than summer.\n   \
        - Avoid August: Extreme heat, many locals on holiday.\n\n\
        3. **Daily Budget Estimates (per person)**\n   \
        - Budget: $80–$120/day (hostel, street food, free sights)\n   \
        - Mid-range: $150–$250/day (3-star hotel, restaurants, museums)\n   \
        - Luxury: $400+/day\n\n\
        4. **Key Tips**\n   \
        - Roma Pass ($32): Unlimited metro + museum discounts.\n   \
        - Pre-book Vatican and Colosseum tickets (sells out 2+ weeks ahead).\n   \
        - 'Coperto' table cover charge (~€3) is normal at restaurants.",
    )
}
