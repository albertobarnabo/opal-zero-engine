use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::env;
use crate::protocol::Tool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResponse {
    Text(String),
    ToolCall { name: String, arguments: String },
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn generate_response(&self, prompt: &str, tools: Option<Vec<Tool>>) -> Result<ToolResponse, String>;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
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

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
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
#[serde(untagged)]
enum ResponseMessage {
    Text {
        content: Option<String>,
    },
    ToolCall {
        tool_calls: Option<Vec<ToolCall>>,
        #[serde(default)]
        #[allow(dead_code)]
        content: Option<String>,
    },
}

#[derive(Deserialize)]
struct ToolCall {
    #[allow(dead_code)]
    id: String,
    function: ToolCallFunction,
}

#[derive(Deserialize)]
struct ToolCallFunction {
    name: String,
    arguments: String,
}

#[async_trait]
impl AiProvider for OpenAIProvider {
    async fn generate_response(&self, prompt: &str, tools: Option<Vec<Tool>>) -> Result<ToolResponse, String> {
        let client = reqwest::Client::new();
        
        let openai_tools = tools.map(|tool_list| {
            tool_list.into_iter().map(|tool| {
                OpenAITool {
                    r#type: "function".to_string(),
                    function: OpenAIFunction {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: serde_json::to_value(&tool.parameters).unwrap_or_default(),
                    },
                }
            }).collect()
        });

        let request_body = OpenAIRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: 0.7,
            tools: openai_tools.clone(),
            tool_choice: openai_tools.as_ref().map(|_| "auto".to_string()),
        };

        let response = client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API returned status {}: {}", status, body));
        }

        let response_body: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        match response_body.choices.first() {
            Some(Choice { message: ResponseMessage::Text { content: Some(text) } }) => {
                Ok(ToolResponse::Text(text.clone()))
            }
            Some(Choice { message: ResponseMessage::ToolCall { tool_calls: Some(calls), .. } }) => {
                if let Some(call) = calls.first() {
                    Ok(ToolResponse::ToolCall {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    })
                } else {
                    Err("Tool calls array is empty".to_string())
                }
            }
            Some(Choice { message: ResponseMessage::Text { content: None } }) => {
                Err("Empty response from OpenAI".to_string())
            }
            None => Err("No choices in response".to_string()),
            _ => Err("Unexpected response format".to_string()),
        }
    }
}
