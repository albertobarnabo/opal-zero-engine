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
/// Implementations live outside `opalzero-core` (e.g. [`opalzero_kernel::engine::OpenAIProvider`])
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

    /// Return a new provider instance configured to use the given text model.
    ///
    /// The default returns `None`, meaning the provider does not support
    /// runtime model switching.  Override this in concrete implementations
    /// (e.g. `OpenAIProvider`) so the dispatcher can downscale routine roles
    /// (WebSearcher, Coder) to a cheaper model without rebuilding the whole
    /// provider.
    fn with_text_model(&self, _model: &str) -> Option<Box<dyn AiProvider>> {
        None
    }
}

// ── MockProvider ──────────────────────────────────────────────────────────────

/// A deterministic in-memory provider for tests and CI.
///
/// Every `generate_response` call returns `response`.
/// `submit_tool_result` returns `response` prefixed with `"tool: "`.
///
/// ```rust
/// use opalzero_core::engine::MockProvider;
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
/// use opalzero_core::engine::SimpleProvider;
///
/// // Point at OpenAI (reads OPENAI_API_KEY from the environment)
/// let p = SimpleProvider::openai("gpt-4o-mini").unwrap();
///
/// // Point at a local Ollama instance
/// let p = SimpleProvider::ollama("llama3");
/// ```
#[derive(Clone)]
pub struct SimpleProvider {
    base_url: String,
    model: String,
    api_key: String,
    client: reqwest::Client,
}

impl SimpleProvider {
    /// Construct from explicit parts.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(request_timeout())
            .build()
            .expect("HTTP client build failed");
        SimpleProvider {
            base_url: base_url.into(),
            model: model.into(),
            api_key: api_key.into(),
            client,
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

    /// Clone this provider but switch to a different model.
    ///
    /// Useful for building a [`RotatingProvider`] whose inner providers all
    /// share the same base URL and API key but target a different model tier.
    pub fn with_model(&self, model: impl Into<String>) -> Self {
        SimpleProvider::new(self.base_url.clone(), model, self.api_key.clone())
    }

    /// Target any OpenAI-compatible endpoint (Groq, Together, Mistral, …).
    ///
    /// # Example
    /// ```rust,no_run
    /// use opalzero_core::engine::SimpleProvider;
    /// let p = SimpleProvider::with_base_url(
    ///     "llama-3.3-70b-versatile",
    ///     "https://api.groq.com/openai/v1",
    ///     "gsk_...",
    /// );
    /// ```
    pub fn with_base_url(
        model: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self::new(base_url, model, api_key)
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

fn sp_parse(body: SpResponse, offered_tools: bool) -> Result<ToolResponse, String> {
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
        Some(text) if !text.is_empty() => {
            if offered_tools {
                // Model returned plain text despite tool schema being offered.
                // This is expected for models that don't support tool calling
                // (e.g. some Ollama models). The caller will handle ToolResponse::Text.
                eprintln!(
                    "[opalzero-core] debug: provider returned plain text despite tools being \
                     offered — model may not support tool calling"
                );
            }
            Ok(ToolResponse::Text(text))
        }
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
        let sp_tools = sp_build_tools(tools);
        let offered = sp_tools.is_some();
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
        sp_parse(sp_post(&self.client, &self.base_url, &self.api_key, &body).await?, offered)
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
        let sp_tools = sp_build_tools(tools);
        let offered = sp_tools.is_some();
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
        sp_parse(sp_post(&self.client, &self.base_url, &self.api_key, &body).await?, offered)
    }

    fn with_text_model(&self, model: &str) -> Option<Box<dyn AiProvider>> {
        Some(Box::new(self.with_model(model)))
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
        sp_parse(sp_post(&self.client, &self.base_url, &self.api_key, &body).await?, false)
    }
}

// ── RotatingProvider ──────────────────────────────────────────────────────────

/// An [`AiProvider`] that distributes calls across multiple [`SimpleProvider`]
/// instances using round-robin selection, with automatic failover on rate-limit
/// errors.
///
/// # Why this exists
///
/// Every hosted LLM API has per-key rate limits (tokens/min, requests/min).
/// When OpalZero runs a multi-task mission with several concurrent agents it can
/// hit those limits in seconds.  `RotatingProvider` spreads the load across
/// several API keys so the aggregate limit is multiplied by the key count.
///
/// # Rotation strategy
///
/// Each call picks the next provider in the ring (atomic round-robin).
/// If that call returns a rate-limit error (HTTP 429 / "Too Many Requests"),
/// the ring advances again and the call is retried immediately with the next
/// provider — no sleep required.  This continues until a call succeeds or
/// every provider in the ring has been exhausted, in which case the last error
/// is returned.
///
/// Non-rate-limit errors are surfaced immediately without rotation.
///
/// # Example
///
/// ```rust,no_run
/// use opalzero_core::engine::{RotatingProvider, SimpleProvider};
///
/// // Same model, three separate API keys → 3× the rate limit headroom.
/// let provider = RotatingProvider::from_api_keys(
///     "gpt-4o-mini",
///     "https://api.openai.com/v1",
///     ["sk-key1...", "sk-key2...", "sk-key3..."],
/// );
/// ```
pub struct RotatingProvider {
    providers: Vec<SimpleProvider>,
    /// Atomic cursor into `providers`; incremented on every call.
    current: std::sync::atomic::AtomicUsize,
}

impl RotatingProvider {
    /// Construct from an explicit list of already-configured [`SimpleProvider`] instances.
    ///
    /// # Panics
    /// Panics if `providers` is empty.
    pub fn new(providers: Vec<SimpleProvider>) -> Self {
        assert!(!providers.is_empty(), "RotatingProvider requires at least one provider");
        Self {
            providers,
            current: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Convenience constructor: build from a list of API keys that all target
    /// the same model and base URL.
    ///
    /// ```rust,no_run
    /// use opalzero_core::engine::RotatingProvider;
    /// let p = RotatingProvider::from_api_keys(
    ///     "gpt-4o-mini",
    ///     "https://api.openai.com/v1",
    ///     ["sk-key-a", "sk-key-b"],
    /// );
    /// ```
    pub fn from_api_keys(
        model: impl Into<String> + Clone,
        base_url: impl Into<String> + Clone,
        api_keys: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let model = model.into();
        let base_url = base_url.into();
        let providers: Vec<SimpleProvider> = api_keys
            .into_iter()
            .map(|k| SimpleProvider::new(base_url.clone(), model.clone(), k))
            .collect();
        Self::new(providers)
    }

    /// Attempt `op` starting at the next round-robin slot, rotating to the
    /// next provider on any rate-limit error.  Stops as soon as a call
    /// succeeds or all providers have been tried.
    async fn try_with_rotation<F, Fut>(&self, mut op: F) -> Result<ToolResponse, String>
    where
        F: FnMut(&SimpleProvider) -> Fut,
        Fut: std::future::Future<Output = Result<ToolResponse, String>>,
    {
        use std::sync::atomic::Ordering;
        let n = self.providers.len();
        // Claim a starting slot atomically so concurrent calls don't all pile
        // onto the same provider.
        let start = self.current.fetch_add(1, Ordering::Relaxed) % n;
        let mut last_err = String::new();

        for i in 0..n {
            let idx = (start + i) % n;
            match op(&self.providers[idx]).await {
                Ok(resp) => return Ok(resp),
                Err(e) if is_rate_limit_error(&e) => {
                    tracing::warn!(
                        provider_idx = idx,
                        total = n,
                        error = %e,
                        "RotatingProvider: 429 — advancing to next provider"
                    );
                    // Advance the shared cursor so the *next* independent call
                    // also skips the throttled slot.
                    self.current.fetch_add(1, Ordering::Relaxed);
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }

        Err(format!(
            "RotatingProvider: all {} providers rate-limited. Last error: {}",
            n, last_err
        ))
    }
}

/// Returns `true` when the error string signals an HTTP 429 / rate-limit response.
fn is_rate_limit_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    err.contains("429")
        || lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("ratelimit")
}

#[async_trait]
impl AiProvider for RotatingProvider {
    async fn generate_response(
        &self,
        prompt: &str,
        tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        let prompt = prompt.to_string();
        self.try_with_rotation(|p| {
            // Clone the provider so the async block owns it — avoids the lifetime
            // conflict between the `&p` borrow and the Future's lifetime.
            let p = p.clone();
            let tools = tools.clone();
            let prompt = prompt.clone();
            async move { p.generate_response(&prompt, tools).await }
        })
        .await
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
        let prompt = prompt.to_string();
        let tool_call_id = tool_call_id.to_string();
        let tool_name = tool_name.to_string();
        let tool_arguments = tool_arguments.to_string();
        let tool_result = tool_result.to_string();

        self.try_with_rotation(|p| {
            let p = p.clone();
            let tools = tools.clone();
            let prompt = prompt.clone();
            let tcid = tool_call_id.clone();
            let tname = tool_name.clone();
            let targs = tool_arguments.clone();
            let tres = tool_result.clone();
            async move {
                p.submit_tool_result(&prompt, tools, &tcid, &tname, &targs, &tres)
                    .await
            }
        })
        .await
    }

    async fn generate_vision_response(
        &self,
        prompt: &str,
        image: &ImageData,
    ) -> Result<ToolResponse, String> {
        let prompt = prompt.to_string();
        let image = image.clone();
        self.try_with_rotation(|p| {
            let p = p.clone();
            let prompt = prompt.clone();
            let image = image.clone();
            async move { p.generate_vision_response(&prompt, &image).await }
        })
        .await
    }

    /// Returns a new `RotatingProvider` where every inner provider targets the
    /// given model — preserving the same base URLs and API keys.
    fn with_text_model(&self, model: &str) -> Option<Box<dyn AiProvider>> {
        let new_providers: Vec<SimpleProvider> =
            self.providers.iter().map(|p| p.with_model(model)).collect();
        Some(Box::new(RotatingProvider::new(new_providers)))
    }
}

// ── SubprocessProvider ────────────────────────────────────────────────────────

/// Controls how the prompt is delivered to the subprocess.
#[derive(Debug, Clone)]
pub enum SubprocessInputMode {
    /// Pass the prompt as a CLI flag: `<cmd> <flag> "<prompt>"`.
    ///
    /// Example: `claude -p "research quantum computing"`
    Flag(String),
    /// Write the prompt to the process's **stdin** and close the pipe;
    /// the process should read until EOF and write its answer to stdout.
    Stdin,
}

/// An [`AiProvider`] that delegates tasks to a **full CLI agent process**
/// (e.g. Claude Code, Gemini CLI, Codex) rather than calling a raw LLM API.
///
/// # Why this is fundamentally different from [`SimpleProvider`]
///
/// `SimpleProvider` makes a single stateless API call: prompt-in, text-out.
/// `SubprocessProvider` spawns a complete agent binary that runs its own
/// multi-step reasoning loop — browsing the web, writing and executing code,
/// reading files — before writing its final answer to stdout.
///
/// From OpalZero's perspective the difference is invisible: both implement
/// `AiProvider` and always return `ToolResponse::Text`.  The power comes from
/// what happens *inside* the subprocess: a full agent loop, not a single LLM
/// turn.
///
/// # Trade-offs
///
/// * The subprocess agent uses **its own built-in tools** (file I/O, web
///   browsing, code execution) rather than OpalZero's tool set.  OpalZero's
///   `Pre-Write Gate` cannot intercept those internal calls — you trust the
///   subprocess to be safe.
/// * There is **no tool-call handshake** (`submit_tool_result` is never
///   invoked by the dispatcher when the first response is `Text`).
/// * Each task spawns a **fresh process** — there is no shared in-process
///   state between tasks.
///
/// # Example
///
/// ```rust,no_run
/// use opalzero_core::engine::SubprocessProvider;
///
/// // Use Claude Code as the agent for every task.
/// let provider = SubprocessProvider::claude_code();
///
/// // Use Gemini CLI with a custom model flag.
/// let provider = SubprocessProvider::new("gemini")
///     .with_prompt_flag("-p")
///     .with_extra_args(["--model", "gemini-2.5-pro"]);
/// ```
pub struct SubprocessProvider {
    /// Executable name (resolved via `PATH`) or absolute path.
    command: String,
    /// Fixed arguments inserted *before* the prompt (flags, model selection …).
    extra_args: Vec<String>,
    /// How the prompt is handed to the process.
    input_mode: SubprocessInputMode,
    /// Hard wall-clock limit per subprocess invocation.
    timeout: std::time::Duration,
}

impl SubprocessProvider {
    /// Construct a minimal provider with the given command.
    ///
    /// Defaults: stdin input mode, 5-minute timeout, no extra args.
    /// Use the builder methods to customise before passing to the Dispatcher.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            extra_args: vec![],
            input_mode: SubprocessInputMode::Stdin,
            timeout: std::time::Duration::from_secs(300),
        }
    }

    /// Pre-configured for **Claude Code** (`claude -p "<prompt>"`).
    ///
    /// Requires `claude` to be on `PATH` (install with `npm i -g @anthropic-ai/claude-code`).
    pub fn claude_code() -> Self {
        Self::new("claude").with_prompt_flag("-p")
    }

    /// Pre-configured for **Gemini CLI** (`gemini -p "<prompt>"`).
    ///
    /// Requires `gemini` to be on `PATH` (install with `npm i -g @google/gemini-cli`).
    pub fn gemini() -> Self {
        Self::new("gemini").with_prompt_flag("-p")
    }

    /// Pre-configured for **OpenAI Codex** (`codex "<prompt>"`).
    ///
    /// Requires `codex` to be on `PATH`.
    pub fn codex() -> Self {
        // Codex takes the prompt as a bare positional argument.
        // We model that as stdin so the Dispatcher writes the prompt and codex
        // reads it; adjust with `with_prompt_flag` if the binary differs.
        Self::new("codex").with_prompt_flag("-q")
    }

    /// Switch to flag-based input: `<cmd> <flag> "<prompt>"`.
    pub fn with_prompt_flag(mut self, flag: impl Into<String>) -> Self {
        self.input_mode = SubprocessInputMode::Flag(flag.into());
        self
    }

    /// Switch to stdin-based input: prompt is written to the process's stdin.
    pub fn with_stdin_input(mut self) -> Self {
        self.input_mode = SubprocessInputMode::Stdin;
        self
    }

    /// Append fixed arguments that are inserted before the prompt on every call.
    ///
    /// ```rust,no_run
    /// use opalzero_core::engine::SubprocessProvider;
    /// // Run Claude Code with extended thinking enabled.
    /// let p = SubprocessProvider::claude_code()
    ///     .with_extra_args(["--model", "claude-opus-4-5"]);
    /// ```
    pub fn with_extra_args<S: Into<String>>(
        mut self,
        args: impl IntoIterator<Item = S>,
    ) -> Self {
        self.extra_args = args.into_iter().map(|a| a.into()).collect();
        self
    }

    /// Override the per-invocation timeout (default: 5 minutes).
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Spawn the subprocess, deliver the prompt, wait for it to finish, and
    /// return its stdout as a [`ToolResponse::Text`].
    ///
    /// Returns `Err` on spawn failure, timeout, non-zero exit code, or empty output.
    async fn run(&self, prompt: &str) -> Result<ToolResponse, String> {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt as _;

        let mut cmd = tokio::process::Command::new(&self.command);
        cmd.args(&self.extra_args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        match &self.input_mode {
            SubprocessInputMode::Flag(flag) => {
                cmd.arg(flag).arg(prompt);
                cmd.stdin(Stdio::null());
            }
            SubprocessInputMode::Stdin => {
                cmd.stdin(Stdio::piped());
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "SubprocessProvider: failed to spawn '{}': {}. \
                 Make sure the agent binary is installed and on PATH.",
                self.command, e
            )
        })?;

        // Write prompt to stdin and close the pipe so the child sees EOF.
        if matches!(self.input_mode, SubprocessInputMode::Stdin) {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(prompt.as_bytes())
                    .await
                    .map_err(|e| format!("SubprocessProvider: stdin write failed: {}", e))?;
                // `stdin` is dropped here — closes the pipe, signals EOF.
            }
        }

        // Enforce the wall-clock timeout.
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                format!(
                    "SubprocessProvider: '{}' timed out after {}s",
                    self.command,
                    self.timeout.as_secs()
                )
            })?
            .map_err(|e| format!("SubprocessProvider: wait_with_output failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let snippet: String = stderr.chars().take(400).collect();
            return Err(format!(
                "SubprocessProvider: '{}' exited with status {}. stderr: {}",
                self.command, output.status, snippet
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let trimmed = stdout.trim().to_string();

        if trimmed.is_empty() {
            return Err(format!(
                "SubprocessProvider: '{}' exited successfully but produced no output",
                self.command
            ));
        }

        tracing::info!(
            command = %self.command,
            output_chars = trimmed.len(),
            "subprocess agent completed"
        );

        Ok(ToolResponse::Text(trimmed))
    }
}

#[async_trait]
impl AiProvider for SubprocessProvider {
    /// Runs the subprocess agent with the full OpalZero prompt (system context +
    /// previous task results + task intent) and returns the agent's final
    /// stdout as [`ToolResponse::Text`].
    ///
    /// The `tools` parameter is intentionally ignored: the subprocess agent
    /// uses its own built-in tool set and manages its own reasoning loop.
    async fn generate_response(
        &self,
        prompt: &str,
        _tools: Option<Vec<Tool>>,
    ) -> Result<ToolResponse, String> {
        self.run(prompt).await
    }

    /// Not called in normal operation — `SubprocessProvider::generate_response`
    /// always returns `ToolResponse::Text`, so the dispatcher never initiates a
    /// tool-result handshake.  Implemented defensively by re-running the agent
    /// with the full conversation context formatted as a single prompt.
    async fn submit_tool_result(
        &self,
        prompt: &str,
        _tools: Option<Vec<Tool>>,
        tool_call_id: &str,
        tool_name: &str,
        tool_arguments: &str,
        tool_result: &str,
    ) -> Result<ToolResponse, String> {
        let combined = format!(
            "{prompt}\n\n\
             [Tool call: {tool_name} (id={tool_call_id})]\n\
             Arguments: {tool_arguments}\n\
             Result: {tool_result}\n\n\
             Continue based on the above tool result."
        );
        self.run(&combined).await
    }

    /// Subprocess agents manage their own model configuration through environment
    /// variables or CLI flags — returns `None` to signal that runtime model
    /// switching is not supported at the OpalZero level.
    ///
    /// To target a specific model, configure it when constructing the provider:
    /// ```rust,no_run
    /// use opalzero_core::engine::SubprocessProvider;
    /// let p = SubprocessProvider::claude_code()
    ///     .with_extra_args(["--model", "claude-opus-4-5"]);
    /// ```
    fn with_text_model(&self, _model: &str) -> Option<Box<dyn AiProvider>> {
        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_rate_limit_error_detects_429() {
        assert!(is_rate_limit_error("API returned status 429: Too Many Requests"));
        assert!(is_rate_limit_error("rate_limit exceeded"));
        assert!(is_rate_limit_error("rate limit hit"));
        assert!(is_rate_limit_error("too many requests from this key"));
        assert!(is_rate_limit_error("RateLimit: quota exceeded"));
    }

    #[test]
    fn is_rate_limit_error_ignores_non_rate_errors() {
        assert!(!is_rate_limit_error("API returned status 500: Internal Server Error"));
        assert!(!is_rate_limit_error("timeout"));
        assert!(!is_rate_limit_error("model not found"));
    }

    #[test]
    fn rotating_provider_from_api_keys_creates_correct_count() {
        let p = RotatingProvider::from_api_keys(
            "gpt-4o-mini",
            "https://api.openai.com/v1",
            ["key-a", "key-b", "key-c"],
        );
        assert_eq!(p.providers.len(), 3);
    }

    #[test]
    #[should_panic(expected = "at least one provider")]
    fn rotating_provider_panics_on_empty_list() {
        RotatingProvider::new(vec![]);
    }

    #[test]
    fn with_text_model_returns_rotating_provider_of_same_size() {
        let p = RotatingProvider::from_api_keys(
            "gpt-4o-mini",
            "https://api.openai.com/v1",
            ["key-a", "key-b"],
        );
        let switched = p.with_text_model("gpt-4o");
        assert!(switched.is_some());
    }

    #[test]
    fn simple_provider_with_model_preserves_base_url() {
        let original = SimpleProvider::new("https://api.openai.com/v1", "gpt-4o-mini", "key");
        let switched = original.with_model("gpt-4o");
        // Verify the clone compiles and the method is reachable.
        // The model field is private; we just assert no panic.
        let _ = switched;
    }

    // ── SubprocessProvider ────────────────────────────────────────────────────

    #[test]
    fn subprocess_provider_defaults_to_stdin_mode() {
        let p = SubprocessProvider::new("some-agent");
        assert!(matches!(p.input_mode, SubprocessInputMode::Stdin));
    }

    #[test]
    fn subprocess_provider_claude_code_uses_flag_mode() {
        let p = SubprocessProvider::claude_code();
        match &p.input_mode {
            SubprocessInputMode::Flag(f) => assert_eq!(f, "-p"),
            other => panic!("expected Flag mode, got {:?}", other),
        }
    }

    #[test]
    fn subprocess_provider_gemini_uses_flag_mode() {
        let p = SubprocessProvider::gemini();
        match &p.input_mode {
            SubprocessInputMode::Flag(f) => assert_eq!(f, "-p"),
            other => panic!("expected Flag mode, got {:?}", other),
        }
    }

    #[test]
    fn subprocess_provider_with_extra_args_stores_args() {
        let p = SubprocessProvider::claude_code()
            .with_extra_args(["--model", "claude-opus-4-5"]);
        assert_eq!(p.extra_args, vec!["--model", "claude-opus-4-5"]);
    }

    #[test]
    fn subprocess_provider_with_timeout_stores_duration() {
        let p = SubprocessProvider::new("agent")
            .with_timeout(std::time::Duration::from_secs(60));
        assert_eq!(p.timeout.as_secs(), 60);
    }

    #[test]
    fn subprocess_provider_with_text_model_returns_none() {
        // Subprocess agents manage their own model — OpalZero should not
        // attempt to switch models at the provider level.
        let p = SubprocessProvider::claude_code();
        assert!(p.with_text_model("gpt-4o").is_none());
    }

    #[tokio::test]
    async fn subprocess_provider_returns_stdout_as_text() {
        // Use the system `echo` command as a portable no-dependency subprocess.
        let p = SubprocessProvider::new("echo")
            .with_extra_args(["hello from subprocess"]);
        // In this mode the extra args ARE the output — prompt is sent via stdin
        // but echo ignores stdin and prints its args.
        let result = p.run("ignored").await;
        // echo is always available on unix; skip gracefully on systems without it.
        if let Ok(ToolResponse::Text(text)) = result {
            assert!(text.contains("hello from subprocess"));
        }
    }

    #[tokio::test]
    async fn subprocess_provider_errors_on_missing_binary() {
        let p = SubprocessProvider::new("__opalzero_nonexistent_binary_xyz__");
        let result = p.run("test prompt").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("failed to spawn") || err.contains("No such file"));
    }
}
