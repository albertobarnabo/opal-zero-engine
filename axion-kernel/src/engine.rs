//! Hardened OpenAI provider for the Axion Kernel.
//!
//! Uses `gpt-4o-mini` for text tasks and `gpt-4o` for multimodal vision
//! requests.  Requires the `OPENAI_API_KEY` environment variable.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;

use axion_core::engine::{AiProvider, ImageData, ToolResponse};
use axion_core::protocol::Tool;

// ── Environment-driven defaults ───────────────────────────────────────────────

fn default_max_tokens() -> u32 {
    std::env::var("AXION_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096)
}

fn default_temperature() -> f32 {
    std::env::var("AXION_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.1)
}

fn request_timeout() -> std::time::Duration {
    let secs = std::env::var("AXION_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(90);
    std::time::Duration::from_secs(secs)
}

// ── Public struct ─────────────────────────────────────────────────────────────

pub struct OpenAIProvider {
    api_key:    String,
    text_model: String,
}

impl OpenAIProvider {
    pub fn new() -> Result<Self, String> {
        let api_key = env::var("OPENAI_API_KEY")
            .map_err(|_| "OPENAI_API_KEY environment variable not set".to_string())?;
        let text_model = env::var("AXION_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Ok(OpenAIProvider { api_key, text_model })
    }

    /// Create a provider and optionally override the text model.
    /// `None` keeps whatever `new()` resolved (env var or hard default).
    pub fn new_with_model(model: Option<String>) -> Result<Self, String> {
        let mut provider = Self::new()?;
        if let Some(m) = model {
            provider.text_model = m;
        }
        Ok(provider)
    }
}

// ── Private HTTP types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrlInner },
}

#[derive(Serialize)]
struct ImageUrlInner {
    url: String,
}

#[derive(Serialize)]
struct Message {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OutboundToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OutboundToolCall {
    id: String,
    r#type: String,
    function: OutboundToolCallFunction,
}

#[derive(Serialize)]
struct OutboundToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize, Clone)]
struct OpenAITool {
    r#type: String,
    function: OpenAIFunction,
}

#[derive(Serialize, Clone)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Deserialize)]
struct ToolCall {
    id: String,
    function: ToolCallFunction,
}

#[derive(Deserialize)]
struct ToolCallFunction {
    name: String,
    arguments: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn build_tools(tools: Option<Vec<Tool>>) -> Option<Vec<OpenAITool>> {
    tools.map(|list| {
        list.into_iter()
            .map(|t| OpenAITool {
                r#type: "function".to_string(),
                function: OpenAIFunction {
                    name: t.name,
                    description: t.description,
                    parameters: serde_json::to_value(&t.parameters).unwrap_or_default(),
                },
            })
            .collect()
    })
}

fn parse_response(body: OpenAIResponse) -> Result<ToolResponse, String> {
    let msg = body
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "No choices in response".to_string())?
        .message;

    if let Some(calls) = msg.tool_calls {
        let call = calls
            .into_iter()
            .next()
            .ok_or_else(|| "Tool calls array is empty".to_string())?;
        return Ok(ToolResponse::ToolCall {
            id: call.id,
            name: call.function.name,
            arguments: call.function.arguments,
        });
    }

    match msg.content {
        Some(text) if !text.is_empty() => Ok(ToolResponse::Text(text)),
        _ => Err("Empty response from OpenAI".to_string()),
    }
}

async fn post_to_openai(
    client: &reqwest::Client,
    api_key: &str,
    body: &OpenAIRequest,
) -> Result<OpenAIResponse, String> {
    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API returned status {}: {}", status, text));
    }

    resp.json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))
}

// ── AiProvider implementation ─────────────────────────────────────────────────

#[async_trait]
impl AiProvider for OpenAIProvider {
    async fn generate_response(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        let openai_tools = build_tools(tools);

        let body = OpenAIRequest {
            model: self.text_model.clone(),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Text(prompt.to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tool_choice: openai_tools.as_ref().map(|_| "auto".to_string()),
            tools: openai_tools,
        };

        parse_response(post_to_openai(&client, &self.api_key, &body).await?)
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
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        let openai_tools = build_tools(tools);

        let body = OpenAIRequest {
            model: self.text_model.clone(),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: Some(MessageContent::Text(prompt.to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                Message {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![OutboundToolCall {
                        id: tool_call_id.to_string(),
                        r#type: "function".to_string(),
                        function: OutboundToolCallFunction {
                            name: tool_name.to_string(),
                            arguments: tool_arguments.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                },
                Message {
                    role: "tool".to_string(),
                    content: Some(MessageContent::Text(tool_result.to_string())),
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id.to_string()),
                },
            ],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tool_choice: openai_tools.as_ref().map(|_| "auto".to_string()),
            tools: openai_tools,
        };

        parse_response(post_to_openai(&client, &self.api_key, &body).await?)
    }

    /// Multimodal request: text prompt + image → gpt-4o vision.
    async fn generate_vision_response(
        &self,
        prompt: &str,
        image: &ImageData,
    ) -> Result<ToolResponse, String> {
        let image_url = if let Some(b64) = &image.base64 {
            format!("data:{};base64,{}", image.mime_type, b64)
        } else if let Some(url) = &image.url {
            url.clone()
        } else {
            return Err("ImageData has neither base64 nor url".to_string());
        };

        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        let body = OpenAIRequest {
            model: "gpt-4o".to_string(), // Vision requires gpt-4o
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(MessageContent::Parts(vec![
                    ContentPart::Text {
                        text: prompt.to_string(),
                    },
                    ContentPart::ImageUrl {
                        image_url: ImageUrlInner { url: image_url },
                    },
                ])),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tools: None,
            tool_choice: None,
        };

        parse_response(post_to_openai(&client, &self.api_key, &body).await?)
    }
}
