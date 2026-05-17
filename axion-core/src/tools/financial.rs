//! Alpha Vantage financial data tools.
//!
//! Four async functions that fetch real market data for a given ticker symbol.
//! Each reads `ALPHA_VANTAGE_API_KEY` from the environment at call time, so the
//! key can be injected per-request via `std::env::set_var` in the server handler.
//!
//! Rate-limit and invalid-key responses from Alpha Vantage are surfaced as
//! `Err(...)` so the LLM receives a clear error message instead of silently
//! empty data.

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Read and validate the API key, returning a clear error if missing.
fn av_api_key() -> Result<String, String> {
    std::env::var("ALPHA_VANTAGE_API_KEY").map_err(|_| {
        "ALPHA_VANTAGE_API_KEY environment variable not set. \
         Obtain a free key at https://www.alphavantage.co/support/#api-key and set it."
            .to_string()
    })
}

/// Check for the two standard Alpha Vantage error envelopes.
fn check_av_errors(data: &serde_json::Value) -> Result<(), String> {
    if let Some(note) = data.get("Note").and_then(|v| v.as_str()) {
        return Err(format!(
            "Alpha Vantage rate limit reached: {}. \
             The free tier allows 25 requests/day and 5 requests/minute.",
            note
        ));
    }
    if let Some(info) = data.get("Information").and_then(|v| v.as_str()) {
        return Err(format!("Alpha Vantage API error: {}", info));
    }
    Ok(())
}

// ── get_company_overview ───────────────────────────────────────────────────────

/// Fetch company fundamentals: name, sector, market cap, P/E, EPS, dividend,
/// 52-week range, and analyst target price.
pub async fn get_company_overview(symbol: &str) -> Result<String, String> {
    let api_key = av_api_key()?;
    let url = format!(
        "https://www.alphavantage.co/query?function=OVERVIEW&symbol={}&apikey={}",
        symbol, api_key
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Alpha Vantage request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Alpha Vantage HTTP error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Alpha Vantage response: {}", e))?;

    check_av_errors(&data)?;

    // Detect an empty response (unknown ticker returns `{}`)
    if data.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        return Err(format!(
            "No data found for symbol '{}'. \
             Verify the ticker is listed on a US exchange supported by Alpha Vantage.",
            symbol
        ));
    }

    let get = |key: &str| {
        data.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("N/A")
            .to_string()
    };

    Ok(format!(
        "Company Overview — {sym} ({name})\n\
         Exchange:        {exchange}\n\
         Sector:          {sector}\n\
         Industry:        {industry}\n\
         Market Cap:      ${market_cap}\n\
         PE Ratio:        {pe}\n\
         EPS:             {eps}\n\
         Dividend Yield:  {div}%\n\
         52-Week High:    ${wk_high}\n\
         52-Week Low:     ${wk_low}\n\
         Analyst Target:  ${target}\n\
         Beta:            {beta}\n\
         Description:     {desc}",
        sym      = symbol.to_uppercase(),
        name     = get("Name"),
        exchange = get("Exchange"),
        sector   = get("Sector"),
        industry = get("Industry"),
        market_cap = get("MarketCapitalization"),
        pe       = get("PERatio"),
        eps      = get("EPS"),
        div      = get("DividendYield"),
        wk_high  = get("52WeekHigh"),
        wk_low   = get("52WeekLow"),
        target   = get("AnalystTargetPrice"),
        beta     = get("Beta"),
        desc     = get("Description"),
    ))
}

// ── get_price_history ─────────────────────────────────────────────────────────

/// Fetch OHLCV daily price history. `period` is either `"compact"` (last 100
/// trading days) or `"full"` (20+ years). Returns the 10 most recent bars.
pub async fn get_price_history(symbol: &str, period: &str) -> Result<String, String> {
    let api_key = av_api_key()?;

    // Alpha Vantage uses "compact" or "full" for the output size parameter
    let output_size = match period.to_lowercase().as_str() {
        "full" => "full",
        _ => "compact", // default for "daily", "1m", "3m", "6m", etc.
    };

    let url = format!(
        "https://www.alphavantage.co/query?function=TIME_SERIES_DAILY&symbol={}\
         &outputsize={}&apikey={}",
        symbol, output_size, api_key
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Alpha Vantage request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Alpha Vantage HTTP error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Alpha Vantage response: {}", e))?;

    check_av_errors(&data)?;

    let series = data
        .get("Time Series (Daily)")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            format!(
                "No price history found for '{}'. \
                 The symbol may be invalid or the market may be closed.",
                symbol
            )
        })?;

    // Collect dates in descending order and take the 10 most recent
    let mut dates: Vec<&str> = series.keys().map(|k| k.as_str()).collect();
    dates.sort_unstable_by(|a, b| b.cmp(a));
    let recent: Vec<&str> = dates.into_iter().take(10).collect();

    let get_field = |day: &serde_json::Value, key: &str| {
        day.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("N/A")
            .to_string()
    };

    let rows: Vec<String> = recent
        .iter()
        .filter_map(|date| {
            let day = series.get(*date)?;
            Some(format!(
                "  {date}  open={open}  high={high}  low={low}  close={close}  vol={vol}",
                date  = date,
                open  = get_field(day, "1. open"),
                high  = get_field(day, "2. high"),
                low   = get_field(day, "3. low"),
                close = get_field(day, "4. close"),
                vol   = get_field(day, "5. volume"),
            ))
        })
        .collect();

    Ok(format!(
        "Daily Price History — {} (last {} trading days)\n{}",
        symbol.to_uppercase(),
        rows.len(),
        rows.join("\n")
    ))
}

// ── get_income_statement ──────────────────────────────────────────────────────

/// Fetch the last 4 annual income statements: revenue, gross profit, operating
/// income, net income, and EPS.
pub async fn get_income_statement(symbol: &str) -> Result<String, String> {
    let api_key = av_api_key()?;
    let url = format!(
        "https://www.alphavantage.co/query?function=INCOME_STATEMENT&symbol={}&apikey={}",
        symbol, api_key
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Alpha Vantage request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Alpha Vantage HTTP error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Alpha Vantage response: {}", e))?;

    check_av_errors(&data)?;

    let reports = data
        .get("annualReports")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!(
                "No income statement data found for '{}'. \
                 This endpoint requires a premium Alpha Vantage plan for some symbols.",
                symbol
            )
        })?;

    let get = |report: &serde_json::Value, key: &str| {
        report
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("N/A")
            .to_string()
    };

    let rows: Vec<String> = reports
        .iter()
        .take(4)
        .map(|r| {
            format!(
                "  FY {fy}  Revenue=${rev}  GrossProfit=${gp}  OperatingIncome=${oi}  \
                 NetIncome=${ni}  EPS={eps}",
                fy  = get(r, "fiscalDateEnding"),
                rev = get(r, "totalRevenue"),
                gp  = get(r, "grossProfit"),
                oi  = get(r, "operatingIncome"),
                ni  = get(r, "netIncome"),
                eps = get(r, "reportedEPS"),
            )
        })
        .collect();

    Ok(format!(
        "Annual Income Statements — {} (last {} fiscal years)\n{}",
        symbol.to_uppercase(),
        rows.len(),
        rows.join("\n")
    ))
}

// ── get_news_sentiment ────────────────────────────────────────────────────────

/// Fetch recent news and sentiment scores for a ticker.
/// Returns up to 5 articles with title, source, time, and overall sentiment.
pub async fn get_news_sentiment(symbol: &str) -> Result<String, String> {
    let api_key = av_api_key()?;
    let url = format!(
        "https://www.alphavantage.co/query?function=NEWS_SENTIMENT&tickers={}&limit=5&apikey={}",
        symbol, api_key
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Alpha Vantage request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Alpha Vantage HTTP error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Alpha Vantage response: {}", e))?;

    check_av_errors(&data)?;

    let feed = data
        .get("feed")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!(
                "No news sentiment data found for '{}'. \
                 The symbol may not have recent news coverage.",
                symbol
            )
        })?;

    if feed.is_empty() {
        return Ok(format!(
            "News Sentiment — {}: No recent news articles found.",
            symbol.to_uppercase()
        ));
    }

    let get = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("N/A")
            .to_string()
    };

    // Overall sentiment from the feed-level summary, if present
    let overall = data
        .get("sentiment_score_definition")
        .and_then(|v| v.as_str())
        .unwrap_or("see per-article scores below");

    let articles: Vec<String> = feed
        .iter()
        .take(5)
        .map(|item| {
            // Per-ticker sentiment within this article
            let ticker_sentiment = item
                .get("ticker_sentiment")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter().find(|ts| {
                        ts.get("ticker")
                            .and_then(|t| t.as_str())
                            .map(|t| t.eq_ignore_ascii_case(symbol))
                            .unwrap_or(false)
                    })
                })
                .and_then(|ts| ts.get("ticker_sentiment_label"))
                .and_then(|v| v.as_str())
                .unwrap_or("N/A");

            format!(
                "  [{sentiment}] {title}\n    Source: {source}  Time: {time}\n    {url}",
                sentiment = ticker_sentiment,
                title     = get(item, "title"),
                source    = get(item, "source"),
                time      = get(item, "time_published"),
                url       = get(item, "url"),
            )
        })
        .collect();

    Ok(format!(
        "News & Sentiment — {} (last {} articles)\n\
         Sentiment scale: {}\n\n{}",
        symbol.to_uppercase(),
        articles.len(),
        overall,
        articles.join("\n\n")
    ))
}
