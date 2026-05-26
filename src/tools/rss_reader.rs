//! `rss_reader` tool — fetch and parse an RSS or Atom feed.
//!
//! Returns the feed title, description, and up to 20 of the most recent items
//! (title, link, summary, and published date).  Works with any standard RSS 2.0
//! or Atom 1.0 feed.
//!
//! Use this for news monitoring, release tracking, blog aggregation, or any
//! mission that needs to consume a structured content stream.

use feed_rs::parser;
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 20;
const FETCH_TIMEOUT_SECS: u64 = 20;

#[derive(Deserialize)]
struct RssArgs {
    /// Full URL of the RSS or Atom feed (https://...).
    url: String,
    /// Maximum number of items to return (default: 20, max: 20).
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize { MAX_ITEMS }

#[derive(Serialize)]
struct FeedResult {
    title:       Option<String>,
    description: Option<String>,
    feed_url:    String,
    item_count:  usize,
    items:       Vec<FeedItem>,
}

#[derive(Serialize)]
struct FeedItem {
    title:     Option<String>,
    link:      Option<String>,
    summary:   Option<String>,
    published: Option<String>,
}

pub async fn execute_rss_reader(arguments: &str) -> Result<String, String> {
    let args: RssArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("rss_reader: invalid arguments: {e}"))?;

    if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
        return Err(format!("rss_reader: URL must start with http:// or https://: {}", args.url));
    }

    let limit = args.limit.min(MAX_ITEMS);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .user_agent("opalzero-agent/1.0 (feed reader)")
        .build()
        .map_err(|e| format!("rss_reader: client build failed: {e}"))?;

    let resp = client
        .get(&args.url)
        .header("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml")
        .send()
        .await
        .map_err(|e| format!("rss_reader: request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("rss_reader: server returned {}", resp.status()));
    }

    let bytes = resp.bytes().await
        .map_err(|e| format!("rss_reader: failed to read body: {e}"))?;

    let feed = parser::parse(bytes.as_ref())
        .map_err(|e| format!("rss_reader: failed to parse feed: {e}"))?;

    let title       = feed.title.map(|t| t.content);
    let description = feed.description.map(|d| d.content);

    let items: Vec<FeedItem> = feed
        .entries
        .into_iter()
        .take(limit)
        .map(|entry| FeedItem {
            title: entry.title.map(|t| t.content),
            link:  entry.links.into_iter().next().map(|l| l.href),
            summary: entry
                .summary
                .map(|s| s.content)
                .or_else(|| entry.content.and_then(|c| c.body)),
            published: entry
                .published
                .or(entry.updated)
                .map(|dt| dt.to_rfc3339()),
        })
        .collect();

    let item_count = items.len();
    let result = FeedResult {
        title,
        description,
        feed_url: args.url,
        item_count,
        items,
    };
    serde_json::to_string(&result)
        .map_err(|e| format!("rss_reader: serialisation failed: {e}"))
}
