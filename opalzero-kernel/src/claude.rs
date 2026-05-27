//! Anthropic Claude provider for the OpalZero Kernel.
//!
//! Uses the Anthropic Messages API (`/v1/messages`).  By default the model is
//! read from `OPALZERO_MODEL` (or falls back to `claude-sonnet-4-5`).
//! Requires the `ANTHROPIC_API_KEY` environment variable.
//!
//! # Provider selection
//! Set `OPALZERO_PROVIDER=claude` to activate this provider in `opalzero-server`.
//!
//! # Model routing
//! Routine roles (WebSearcher, Coder) are automatically downscaled to
//! `claude-haiku-3-5` to reduce cost and latency; reasoning-heavy roles
//! (Planner, Analyst) keep whatever model the user selected.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;

use opalzero_core::engine::{AiProvider, ImageData, ToolResponse};
use opalzero_core::protocol::Tool;

// ── Environment-driven defaults ───────────────────────────────────────────────

fn default_max_tokens() -> u32 {
    std::env::var("OPALZERO_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8192)
}

fn default_temperature() -> f32 {
    std::env::var("OPALZERO_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.1)
}

fn request_timeout() -> std::time::Duration {
    let secs = std::env::var("OPALZERO_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(90);
    std::time::Duration::from_secs(secs)
}

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Cheap model used for routine roles (WebSearcher, Coder).
const HAIKU_MODEL: &str = "claude-haiku-3-5";

// ── Public struct ─────────────────────────────────────────────────────────────

pub struct ClaudeProvider {
    api_key:    String,
    text_model: String,
    client:     reqwest::Client,
}

impl ClaudeProvider {
    pub fn new() -> Result<Self, String> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY environment variable not set".to_string())?;
        let text_model = env::var("OPALZERO_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-5".to_string());
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        Ok(ClaudeProvider { api_key, text_model, client })
    }

    /// Create a provider and optionally override the text model.
    pub fn new_with_model(model: Option<String>) -> Result<Self, String> {
        let mut provider = Self::new()?;
        if let Some(m) = model {
            provider.text_model = m;
        }
        Ok(provider)
    }

    /// Create a provider with an explicit API key, bypassing the environment.
    pub fn new_with_explicit_key(
        model: Option<String>,
        api_key: Option<String>,
    ) -> Result<Self, String> {
        let key = match api_key {
            Some(k) if !k.is_empty() => k,
            _ => env::var("ANTHROPIC_API_KEY")
                .map_err(|_| "ANTHROPIC_API_KEY environment variable not set".to_string())?,
        };
        let text_model = model.unwrap_or_else(|| {
            env::var("OPALZERO_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".to_string())
        });
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        Ok(ClaudeProvider { api_key: key, text_model, client })
    }
}

// ── Request / response types (Anthropic Messages API) ─────────────────────────

#[derive(Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    temperature: f32,
    messages: Vec<ClaudeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ClaudeTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ClaudeToolChoice>,
}

/// A single message in the Anthropic conversation.
/// `content` is polymorphic: either a plain string or an array of content
/// blocks (used for multimodal and tool-result turns).
#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: ClaudeContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ClaudeContent {
    Text(String),
    Blocks(Vec<ClaudeBlock>),
}

/// Individual content block inside a `ClaudeMessage`.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeBlock {
    Text {
        text: String,
    },
    Image {
        source: ClaudeImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeImageSource {
    Base64 {
        media_type: String,
        data: String,
    },
    Url {
        url: String,
    },
}

#[derive(Serialize, Clone)]
struct ClaudeTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize)]
struct ClaudeToolChoice {
    r#type: String,
}

// ── Response types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeRespBlock>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeRespBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_claude_tools(tools: Option<Vec<Tool>>) -> Option<Vec<ClaudeTool>> {
    tools.map(|list| {
        list.into_iter()
            .map(|t| ClaudeTool {
                name: t.name,
                description: t.description,
                input_schema: serde_json::to_value(&t.parameters).unwrap_or_default(),
            })
            .collect()
    })
}

fn parse_claude_response(body: ClaudeResponse) -> Result<ToolResponse, String> {
    // Prefer tool_use blocks; fall back to the first text block.
    for block in &body.content {
        if let ClaudeRespBlock::ToolUse { id, name, input } = block {
            let arguments = serde_json::to_string(input)
                .unwrap_or_else(|_| "{}".to_string());
            return Ok(ToolResponse::ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments,
            });
        }
    }
    for block in body.content {
        if let ClaudeRespBlock::Text { text } = block {
            if !text.is_empty() {
                return Ok(ToolResponse::Text(text));
            }
        }
    }
    Err("Empty response from Claude".to_string())
}

async fn post_to_claude(
    client: &reqwest::Client,
    api_key: &str,
    body: &ClaudeRequest,
) -> Result<ClaudeResponse, String> {
    let resp = client
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API returned status {}: {}", status, text));
    }

    resp.json::<ClaudeResponse>()
        .await
        .map_err(|e| format!("Failed to parse Claude response: {}", e))
}

// ── model_for_role helper ─────────────────────────────────────────────────────

/// Returns the Claude model to use for a given agent role.
///
/// Reasoning-heavy roles (Planner, Analyst) keep the user-selected model.
/// Routine roles (WebSearcher, Coder) are downscaled to `claude-haiku-3-5`
/// to reduce cost and latency.
pub fn model_for_role(selected_model: &str, role: &opalzero_core::protocol::AgentRole) -> String {
    use opalzero_core::protocol::AgentRole;
    match role {
        AgentRole::WebSearcher | AgentRole::Coder => HAIKU_MODEL.to_string(),
        AgentRole::Planner | AgentRole::Analyst   => selected_model.to_string(),
    }
}

// ── AiProvider implementation ─────────────────────────────────────────────────

#[async_trait]
impl AiProvider for ClaudeProvider {
    async fn generate_response(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        let claude_tools = build_claude_tools(tools);
        let tool_choice = claude_tools.as_ref().map(|_| ClaudeToolChoice {
            r#type: "auto".to_string(),
        });

        let body = ClaudeRequest {
            model: self.text_model.clone(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: ClaudeContent::Text(prompt.to_string()),
            }],
            tools: claude_tools,
            tool_choice,
        };

        parse_claude_response(post_to_claude(&self.client, &self.api_key, &body).await?)
    }

    async fn submit_tool_result(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
        tool_call_id: &str,
        tool_name: &str,
        tool_arguments: &str,
        tool_result: &str,
    ) -> Result<ToolResponse, String> {
        let claude_tools = build_claude_tools(tools);
        let tool_choice = claude_tools.as_ref().map(|_| ClaudeToolChoice {
            r#type: "auto".to_string(),
        });

        // Parse the original tool call arguments back into a JSON value so
        // we can reconstruct the assistant's `tool_use` block faithfully.
        let input: serde_json::Value = serde_json::from_str(tool_arguments)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));

        let body = ClaudeRequest {
            model: self.text_model.clone(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            messages: vec![
                // Original user turn
                ClaudeMessage {
                    role: "user".to_string(),
                    content: ClaudeContent::Text(prompt.to_string()),
                },
                // Assistant's tool_use turn
                ClaudeMessage {
                    role: "assistant".to_string(),
                    content: ClaudeContent::Blocks(vec![ClaudeBlock::ToolUse {
                        id: tool_call_id.to_string(),
                        name: tool_name.to_string(),
                        input,
                    }]),
                },
                // User's tool_result turn
                ClaudeMessage {
                    role: "user".to_string(),
                    content: ClaudeContent::Blocks(vec![ClaudeBlock::ToolResult {
                        tool_use_id: tool_call_id.to_string(),
                        content: tool_result.to_string(),
                    }]),
                },
            ],
            tools: claude_tools,
            tool_choice,
        };

        parse_claude_response(post_to_claude(&self.client, &self.api_key, &body).await?)
    }

    /// Vision request: text prompt + image → multimodal message.
    /// Requires a vision-capable Claude model (e.g. `claude-opus-4-5`).
    async fn generate_vision_response(
        &self,
        prompt: &str,
        image: &ImageData,
    ) -> Result<ToolResponse, String> {
        let image_block = if let Some(b64) = &image.base64 {
            ClaudeBlock::Image {
                source: ClaudeImageSource::Base64 {
                    media_type: image.mime_type.clone(),
                    data: b64.clone(),
                },
            }
        } else if let Some(url) = &image.url {
            ClaudeBlock::Image {
                source: ClaudeImageSource::Url { url: url.clone() },
            }
        } else {
            return Err("ImageData has neither base64 nor url".to_string());
        };

        let body = ClaudeRequest {
            model: self.text_model.clone(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: ClaudeContent::Blocks(vec![
                    image_block,
                    ClaudeBlock::Text { text: prompt.to_string() },
                ]),
            }],
            tools: None,
            tool_choice: None,
        };

        parse_claude_response(post_to_claude(&self.client, &self.api_key, &body).await?)
    }

    fn with_text_model(&self, model: &str) -> Option<Box<dyn AiProvider>> {
        Some(Box::new(ClaudeProvider {
            api_key:    self.api_key.clone(),
            text_model: model.to_string(),
            client:     self.client.clone(),
        }))
    }
}
