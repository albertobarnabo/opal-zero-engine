//! `fetch_page` tool — fetch a URL and return its readable text content.
//!
//! Covers the 80% use-case for "browser automation": reading a webpage that
//! the web_search tool doesn't index — a pricing page, job listing, docs page,
//! or any URL the agent has already found.  No Playwright binary required.
//!
//! Pipeline:
//!   1. Fetch the raw HTML via reqwest.
//!   2. Strip scripts, styles, nav, and boilerplate tags.
//!   3. Collapse whitespace and return clean prose.
//!
//! Output is capped at 8 000 chars (~2 000 tokens) so agents stay within
//! context budget even on large pages.

use serde::Deserialize;

const MAX_OUTPUT_CHARS: usize = 8_000;
const FETCH_TIMEOUT_SECS: u64 = 20;

#[derive(Deserialize)]
struct FetchPageArgs {
    /// The URL to fetch (must start with http:// or https://).
    url: String,
    /// Optional hint — what the agent is looking for on the page.
    /// Not used for filtering yet, reserved for future semantic extraction.
    #[serde(default)]
    #[allow(dead_code)]
    prompt: Option<String>,
}

pub async fn execute_fetch_page(arguments: &str) -> Result<String, String> {
    let args: FetchPageArgs = serde_json::from_str(arguments)
        .map_err(|e| format!("fetch_page: invalid arguments: {e}"))?;

    if !args.url.starts_with("http://") && !args.url.starts_with("https://") {
        return Err(format!(
            "fetch_page: URL must start with http:// or https://: {}",
            args.url
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .user_agent("axion-agent/1.0 (research assistant)")
        .build()
        .map_err(|e| format!("fetch_page: client build failed: {e}"))?;

    let resp = client
        .get(&args.url)
        .header("Accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| format!("fetch_page: request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("fetch_page: server returned {}", status));
    }

    // Only process text/* content types — skip binaries.
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !content_type.contains("text/") && !content_type.contains("html") {
        return Err(format!(
            "fetch_page: unsupported content-type '{}' (expected HTML)",
            content_type
        ));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("fetch_page: failed to read body: {e}"))?;

    let text = html_to_text(&html);
    let out = if text.chars().count() > MAX_OUTPUT_CHARS {
        let end = text.floor_char_boundary(MAX_OUTPUT_CHARS);
        format!("{}… [truncated — page had more content]", &text[..end])
    } else {
        text
    };

    Ok(out)
}

// ── HTML → plain text ─────────────────────────────────────────────────────────

/// Strip HTML and return readable prose.
///
/// Removes entire subtrees of tags that never contain readable content
/// (script, style, nav, header, footer, aside, svg, form, button), then
/// strips remaining tags and cleans up whitespace.
fn html_to_text(html: &str) -> String {
    // 1. Remove full subtrees that carry no readable content.
    let blocked = &[
        "script", "style", "nav", "header", "footer", "aside",
        "svg", "form", "button", "noscript", "iframe", "meta", "head",
    ];
    let mut s = html.to_string();
    for tag in blocked {
        s = remove_tag_subtree(&s, tag);
    }

    // 2. Replace block elements with newlines so paragraphs are preserved.
    let block_tags = &[
        "p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6",
        "article", "section", "main", "blockquote", "pre",
    ];
    for tag in block_tags {
        let open  = format!("<{}", tag);
        let close = format!("</{}", tag);
        s = s.replace(&open,  &format!("\n<{}", tag));
        s = s.replace(&close, &format!("\n</{}", tag));
    }

    // 3. Strip all remaining HTML tags.
    s = strip_tags(&s);

    // 4. Decode common HTML entities.
    s = decode_entities(&s);

    // 5. Collapse whitespace — normalise runs of blanks/newlines.
    let mut result = String::with_capacity(s.len());
    let mut last_newline = true;
    let mut blank_lines  = 0u32;
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_lines += 1;
            if blank_lines <= 1 && !last_newline {
                result.push('\n');
                last_newline = true;
            }
        } else {
            result.push_str(trimmed);
            result.push('\n');
            last_newline = false;
            blank_lines  = 0;
        }
    }

    result.trim().to_string()
}

/// Remove `<tag ...>...</tag>` subtrees (case-insensitive, handles nesting).
fn remove_tag_subtree(html: &str, tag: &str) -> String {
    let open_pat  = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut result = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut pos = 0;

    while pos < html.len() {
        if let Some(start) = lower[pos..].find(&open_pat) {
            let abs_start = pos + start;
            // Confirm it's a real tag (next char is > or space).
            let after = lower[abs_start + open_pat.len()..].chars().next();
            if !matches!(after, Some('>') | Some(' ') | Some('\n') | Some('\t') | Some('/')) {
                result.push_str(&html[pos..abs_start + open_pat.len()]);
                pos = abs_start + open_pat.len();
                continue;
            }
            // Emit everything before this tag.
            result.push_str(&html[pos..abs_start]);
            // Find matching close tag (skip nested open tags of same name).
            let mut depth = 1usize;
            let mut scan  = abs_start + open_pat.len();
            while depth > 0 && scan < html.len() {
                let sub = &lower[scan..];
                let next_open  = sub.find(&open_pat);
                let next_close = sub.find(&close_pat);
                match (next_open, next_close) {
                    (Some(o), Some(c)) if o < c => { depth += 1; scan += o + open_pat.len(); }
                    (_, Some(c)) => {
                        depth -= 1;
                        scan += c + close_pat.len();
                    }
                    _ => break,
                }
            }
            pos = scan;
        } else {
            result.push_str(&html[pos..]);
            break;
        }
    }
    result
}

/// Strip all remaining `<...>` tags.
fn strip_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => { in_tag = true; }
            '>' => { in_tag = false; result.push(' '); }
            _   => { if !in_tag { result.push(ch); } }
        }
    }
    result
}

/// Decode a small set of common HTML entities.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;",  "&")
     .replace("&lt;",   "<")
     .replace("&gt;",   ">")
     .replace("&quot;", "\"")
     .replace("&#39;",  "'")
     .replace("&apos;", "'")
     .replace("&nbsp;", " ")
     .replace("&mdash;", "—")
     .replace("&ndash;", "–")
     .replace("&hellip;", "…")
}
