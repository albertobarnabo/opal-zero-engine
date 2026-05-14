use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::protocol::Tool;

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

// ── Multimodal image carrier ──────────────────────────────────────────────────

/// Image payload for vision-capable requests.
///
/// Exactly one of `base64` or `url` must be set.
/// `mime_type` is required in both cases (e.g. `"image/png"`, `"image/jpeg"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    /// Raw image bytes encoded as standard base64 (no data-URI prefix).
    pub base64: Option<String>,
    /// Publicly accessible image URL (alternative to base64).
    pub url: Option<String>,
    /// MIME type, e.g. `"image/png"` or `"image/jpeg"`.
    pub mime_type: String,
}

// ── Core response type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResponse {
    Text(String),
    ToolCall { id: String, name: String, arguments: String },
}

// ── AiProvider trait (public interface) ──────────────────────────────────────

/// Core interface every AI backend must implement.
///
/// Implementations live outside `axion-core` (e.g. [`axion_kernel::engine::OpenAIProvider`])
/// so the library stays decoupled from any specific model or vendor.
/// For local/compatible backends use [`SimpleProvider`]; for testing use [`MockProvider`].
#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn generate_response(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String>;

    async fn submit_tool_result(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
        tool_call_id: &str,
        tool_name: &str,
        tool_arguments: &str,
        tool_result: &str,
    ) -> Result<ToolResponse, String>;

    /// Send a multimodal request containing text + an image to a vision-capable
    /// model.  The default implementation returns an error so providers that do
    /// not support vision do not need to override this method.
    async fn generate_vision_response(
        &self,
        prompt: &str,
        image: &ImageData,
    ) -> Result<ToolResponse, String> {
        let _ = (prompt, image);
        Err("Vision analysis is not supported by this provider".to_string())
    }
}

// ── MockProvider ──────────────────────────────────────────────────────────────

/// A deterministic in-memory provider for tests and CI.
///
/// Every `generate_response` call returns `response`.
/// `submit_tool_result` returns `response` prefixed with `"tool: "`.
///
/// ```rust
/// use axion_core::engine::MockProvider;
/// let p = MockProvider::new("hello world");
/// ```
pub struct MockProvider {
    pub response: String,
}

impl MockProvider {
    pub fn new(response: impl Into<String>) -> Self {
        MockProvider { response: response.into() }
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    async fn generate_response(
        &self,
        _prompt: &str,
        _tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        Ok(ToolResponse::Text(self.response.clone()))
    }

    async fn submit_tool_result(
        &self,
        _prompt: &str,
        _tools: Option<Vec<Tool>>,
        _tool_call_id: &str,
        _tool_name: &str,
        _tool_arguments: &str,
        _tool_result: &str,
    ) -> Result<ToolResponse, String> {
        Ok(ToolResponse::Text(format!("tool: {}", self.response)))
    }
}

// ── SimpleProvider ────────────────────────────────────────────────────────────

/// A configurable provider for any OpenAI-compatible HTTP API.
///
/// Works with OpenAI, Azure OpenAI, Groq, Mistral, and local runtimes such as
/// [Ollama](https://ollama.com/) (which exposes an OpenAI-compatible endpoint
/// at `http://localhost:11434/v1`).
///
/// # Quick start
/// ```rust,no_run
/// use axion_core::engine::SimpleProvider;
///
/// // Point at OpenAI (reads OPENAI_API_KEY from the environment)
/// let p = SimpleProvider::openai("gpt-4o-mini").unwrap();
///
/// // Point at a local Ollama instance
/// let p = SimpleProvider::ollama("llama3");
/// ```
pub struct SimpleProvider {
    base_url: String,
    model: String,
    api_key: String,
}

impl SimpleProvider {
    /// Construct from explicit parts.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        SimpleProvider {
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
        }
    }

    /// Target the real OpenAI API, reading `OPENAI_API_KEY` from the
    /// environment.
    pub fn openai(model: impl Into<String>) -> Result<Self, String> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| "OPENAI_API_KEY environment variable not set".to_string())?;
        Ok(Self::new("https://api.openai.com/v1", model, api_key))
    }

    /// Target a local [Ollama](https://ollama.com/) instance (no API key
    /// required).
    pub fn ollama(model: impl Into<String>) -> Self {
        Self::new("http://localhost:11434/v1", model, "")
    }
}

// ── Private HTTP structs for SimpleProvider ───────────────────────────────────

#[derive(Serialize)]
struct SpRequest {
    model: String,
    messages: Vec<SpMessage>,
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<SpTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum SpContent {
    Text(String),
    Parts(Vec<SpPart>),
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SpPart {
    Text { text: String },
    ImageUrl { image_url: SpImageUrl },
}

#[derive(Serialize)]
struct SpImageUrl {
    url: String,
}

#[derive(Serialize)]
struct SpMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<SpContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<SpOutboundCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct SpOutboundCall {
    id: String,
    r#type: String,
    function: SpOutboundFn,
}

#[derive(Serialize)]
struct SpOutboundFn {
    name: String,
    arguments: String,
}

#[derive(Serialize, Clone)]
struct SpTool {
    r#type: String,
    function: SpFunction,
}

#[derive(Serialize, Clone)]
struct SpFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct SpResponse {
    choices: Vec<SpChoice>,
}

#[derive(Deserialize)]
struct SpChoice {
    message: SpRespMessage,
}

#[derive(Deserialize)]
struct SpRespMessage {
    content: Option<String>,
    tool_calls: Option<Vec<SpToolCall>>,
}

#[derive(Deserialize)]
struct SpToolCall {
    id: String,
    function: SpToolCallFn,
}

#[derive(Deserialize)]
struct SpToolCallFn {
    name: String,
    arguments: String,
}

// ── SimpleProvider helpers ────────────────────────────────────────────────────

fn sp_build_tools(tools: Option<Vec<Tool>>) -> Option<Vec<SpTool>> {
    tools.map(|list| {
        list.into_iter()
            .map(|t| SpTool {
                r#type: "function".to_string(),
                function: SpFunction {
                    name: t.name,
                    description: t.description,
                    parameters: serde_json::to_value(&t.parameters).unwrap_or_default(),
                },
            })
            .collect()
    })
}

fn sp_parse(body: SpResponse) -> Result<ToolResponse, String> {
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
        _ => Err("Empty response from provider".to_string()),
    }
}

async fn sp_post(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    body: &SpRequest,
) -> Result<SpResponse, String> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut req = client.post(&url).json(body);
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key));
    }
    let resp = req
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

// ── AiProvider impl for SimpleProvider ───────────────────────────────────────

#[async_trait]
impl AiProvider for SimpleProvider {
    async fn generate_response(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        let sp_tools = sp_build_tools(tools);
        let body = SpRequest {
            model: self.model.clone(),
            messages: vec![SpMessage {
                role: "user".to_string(),
                content: Some(SpContent::Text(prompt.to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tool_choice: sp_tools.as_ref().map(|_| "auto".to_string()),
            tools: sp_tools,
        };
        sp_parse(sp_post(&client, &self.base_url, &self.api_key, &body).await?)
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
        let sp_tools = sp_build_tools(tools);
        let body = SpRequest {
            model: self.model.clone(),
            messages: vec![
                SpMessage {
                    role: "user".to_string(),
                    content: Some(SpContent::Text(prompt.to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                },
                SpMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![SpOutboundCall {
                        id: tool_call_id.to_string(),
                        r#type: "function".to_string(),
                        function: SpOutboundFn {
                            name: tool_name.to_string(),
                            arguments: tool_arguments.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                },
                SpMessage {
                    role: "tool".to_string(),
                    content: Some(SpContent::Text(tool_result.to_string())),
                    tool_calls: None,
                    tool_call_id: Some(tool_call_id.to_string()),
                },
            ],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tool_choice: sp_tools.as_ref().map(|_| "auto".to_string()),
            tools: sp_tools,
        };
        sp_parse(sp_post(&client, &self.base_url, &self.api_key, &body).await?)
    }

    /// Vision request using the multimodal chat completions format.
    /// Requires the configured model to support image inputs (e.g. `gpt-4o`).
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
        let body = SpRequest {
            model: self.model.clone(),
            messages: vec![SpMessage {
                role: "user".to_string(),
                content: Some(SpContent::Parts(vec![
                    SpPart::Text { text: prompt.to_string() },
                    SpPart::ImageUrl { image_url: SpImageUrl { url: image_url } },
                ])),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            tools: None,
            tool_choice: None,
        };
        sp_parse(sp_post(&client, &self.base_url, &self.api_key, &body).await?)
    }
}
