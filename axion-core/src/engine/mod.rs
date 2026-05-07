use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;
use crate::protocol::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResponse {
    Text(String),
    ToolCall { id: String, name: String, arguments: String },
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn generate_response(&self, prompt: &str, tools: Option<Vec<Tool>>) -> Result<ToolResponse, String>;
    async fn submit_tool_result(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
        tool_call_id: &str,
        tool_name: &str,
        tool_arguments: &str,
        tool_result: &str,
    ) -> Result<ToolResponse, String>;
}

pub struct OpenAIProvider {
    api_key: String,
}

impl OpenAIProvider {
    pub fn new() -> Result<Self, String> {
        let api_key = env::var("OPENAI_API_KEY")
            .map_err(|_| "OPENAI_API_KEY environment variable not set".to_string())?;
        Ok(OpenAIProvider { api_key })
    }
}

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

// Unified message type covering user, assistant, and tool roles.
#[derive(Serialize)]
struct Message {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
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

// Single struct instead of an untagged enum: tool_calls takes priority over content.
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

fn build_tools(tools: Option<Vec<Tool>>) -> Option<Vec<OpenAITool>> {
    tools.map(|list| {
        list.into_iter().map(|t| OpenAITool {
            r#type: "function".to_string(),
            function: OpenAIFunction {
                name: t.name,
                description: t.description,
                parameters: serde_json::to_value(&t.parameters).unwrap_or_default(),
            },
        }).collect()
    })
}

fn parse_response(body: OpenAIResponse) -> Result<ToolResponse, String> {
    let msg = body.choices.into_iter().next()
        .ok_or_else(|| "No choices in response".to_string())?
        .message;

    if let Some(calls) = msg.tool_calls {
        let call = calls.into_iter().next()
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

    resp.json().await.map_err(|e| format!("Failed to parse response: {}", e))
}

#[async_trait]
impl AiProvider for OpenAIProvider {
    async fn generate_response(&self, prompt: &str, tools: Option<Vec<Tool>>) -> Result<ToolResponse, String> {
        let client = reqwest::Client::new();
        let openai_tools = build_tools(tools);

        let body = OpenAIRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: Some(prompt.to_string()),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: 0.0,
            max_tokens: 1024,
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
        let client = reqwest::Client::new();
        let openai_tools = build_tools(tools);

        let body = OpenAIRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: Some(prompt.to_string()),
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
                    content: Some(tool_result.to_string()),
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id.to_string()),
                },
            ],
            temperature: 0.0,
            max_tokens: 1024,
            tool_choice: openai_tools.as_ref().map(|_| "auto".to_string()),
            tools: openai_tools,
        };

        parse_response(post_to_openai(&client, &self.api_key, &body).await?)
    }
}
