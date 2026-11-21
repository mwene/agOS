//! Live model driver for any OpenAI-compatible Chat Completions endpoint.
//!
//! This covers OpenAI, DeepSeek, Groq, Together, Fireworks, vLLM, LM Studio and
//! Ollama's `/v1` compatibility layer. The driver is opt-in: when it is not
//! configured, the kernel keeps using its deterministic built-in drivers, so
//! tests and offline builds never depend on the network.
//!
//! Configuration is read from the environment at startup:
//!
//! | Variable | Meaning | Default |
//! |----------|---------|---------|
//! | `AGOS_LLM_BASE_URL` | API root, e.g. `https://api.openai.com/v1` | `https://api.openai.com/v1` |
//! | `AGOS_LLM_API_KEY` | Bearer token (omit for local runtimes) | none |
//! | `AGOS_LLM_MODEL` | Model name | `gpt-4o-mini` |
//! | `AGOS_LLM_EMBEDDING_MODEL` | Model for `/embeddings` | none (endpoint errors) |
//! | `AGOS_LLM_TIMEOUT_SECS` | Buffered request timeout | `60` |
//! | `AGOS_LLM_STREAM_IDLE_TIMEOUT_SECS` | Abort a stream after this long without data | `30` |
//! | `AGOS_LLM_MAX_TOKENS` | `max_tokens` sent to the provider | provider default |
//! | `AGOS_LLM_TEMPERATURE` | Sampling temperature | provider default |
//!
//! `OPENAI_BASE_URL` / `OPENAI_API_KEY` are accepted as fallbacks.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Name a driver is registered under inside the UMI.
pub const DRIVER_NAME: &str = "OpenAICompatible";

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_STREAM_IDLE_TIMEOUT_SECS: u64 = 30;
const MAX_ERROR_BODY: usize = 500;

/// Configuration for an OpenAI-compatible endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub embedding_model: Option<String>,
    pub timeout_secs: u64,
    pub stream_idle_timeout_secs: u64,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl LlmConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        LlmConfig {
            base_url: base_url.into(),
            api_key: None,
            model: DEFAULT_MODEL.to_string(),
            embedding_model: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            stream_idle_timeout_secs: DEFAULT_STREAM_IDLE_TIMEOUT_SECS,
            max_tokens: None,
            temperature: None,
        }
    }

    /// Build a configuration from the process environment.
    ///
    /// Returns `None` when neither a base URL nor an API key is configured,
    /// which keeps the live driver disabled by default.
    pub fn from_env() -> Option<Self> {
        let base_url =
            env_non_empty("AGOS_LLM_BASE_URL").or_else(|| env_non_empty("OPENAI_BASE_URL"));
        let api_key = env_non_empty("AGOS_LLM_API_KEY").or_else(|| env_non_empty("OPENAI_API_KEY"));

        if base_url.is_none() && api_key.is_none() {
            return None;
        }

        let model = env_non_empty("AGOS_LLM_MODEL").unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let embedding_model = env_non_empty("AGOS_LLM_EMBEDDING_MODEL");
        let timeout_secs = env_non_empty("AGOS_LLM_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let stream_idle_timeout_secs = env_non_empty("AGOS_LLM_STREAM_IDLE_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_STREAM_IDLE_TIMEOUT_SECS);
        let max_tokens = env_non_empty("AGOS_LLM_MAX_TOKENS").and_then(|v| v.parse().ok());
        let temperature = env_non_empty("AGOS_LLM_TEMPERATURE").and_then(|v| v.parse().ok());

        Some(LlmConfig {
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key,
            model,
            embedding_model,
            timeout_secs,
            stream_idle_timeout_secs,
            max_tokens,
            temperature,
        })
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_embedding_model(mut self, model: impl Into<String>) -> Self {
        self.embedding_model = Some(model.into());
        self
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn with_stream_idle_timeout_secs(mut self, secs: u64) -> Self {
        self.stream_idle_timeout_secs = secs;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Full URL of the Chat Completions endpoint.
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Full URL of the embeddings endpoint.
    pub fn embeddings_endpoint(&self) -> String {
        format!("{}/embeddings", self.base_url.trim_end_matches('/'))
    }

    /// Configuration safe to return over the API (the key is masked).
    pub fn redacted(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("base_url".into(), self.base_url.clone());
        map.insert("model".into(), self.model.clone());
        map.insert("timeout_secs".into(), self.timeout_secs.to_string());
        map.insert(
            "stream_idle_timeout_secs".into(),
            self.stream_idle_timeout_secs.to_string(),
        );
        if let Some(model) = &self.embedding_model {
            map.insert("embedding_model".into(), model.clone());
        }
        if let Some(max) = self.max_tokens {
            map.insert("max_tokens".into(), max.to_string());
        }
        if let Some(temp) = self.temperature {
            map.insert("temperature".into(), temp.to_string());
        }
        if self.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
            map.insert("api_key".into(), "***".into());
        }
        map
    }

    /// Apply a partial config update, validating every field first.
    pub fn apply_updates(&mut self, updates: &HashMap<String, String>) -> Result<(), String> {
        let mut next = self.clone();
        for (key, value) in updates {
            match key.as_str() {
                "base_url" => {
                    if value.trim().is_empty() {
                        return Err("base_url cannot be empty".into());
                    }
                    next.base_url = value.trim().to_string();
                }
                "model" => {
                    if value.trim().is_empty() {
                        return Err("model cannot be empty".into());
                    }
                    next.model = value.trim().to_string();
                }
                "api_key" => {
                    next.api_key = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value.trim().to_string())
                    };
                }
                "embedding_model" => {
                    next.embedding_model = if value.trim().is_empty() {
                        None
                    } else {
                        Some(value.trim().to_string())
                    };
                }
                "timeout_secs" => {
                    let secs: u64 = value.parse().map_err(|_| {
                        format!("timeout_secs must be a positive integer, got '{value}'")
                    })?;
                    if secs == 0 {
                        return Err("timeout_secs must be greater than zero".into());
                    }
                    next.timeout_secs = secs;
                }
                "stream_idle_timeout_secs" => {
                    let secs: u64 = value.parse().map_err(|_| {
                        format!(
                            "stream_idle_timeout_secs must be a positive integer, got '{value}'"
                        )
                    })?;
                    if secs == 0 {
                        return Err("stream_idle_timeout_secs must be greater than zero".into());
                    }
                    next.stream_idle_timeout_secs = secs;
                }
                "max_tokens" => {
                    next.max_tokens =
                        Some(value.parse().map_err(|_| {
                            format!("max_tokens must be an integer, got '{value}'")
                        })?);
                }
                "temperature" => {
                    let temp: f32 = value
                        .parse()
                        .map_err(|_| format!("temperature must be a number, got '{value}'"))?;
                    if !(0.0..=2.0).contains(&temp) {
                        return Err("temperature must be between 0.0 and 2.0".into());
                    }
                    next.temperature = Some(temp);
                }
                other => return Err(format!("unknown config key '{other}'")),
            }
        }
        *self = next;
        Ok(())
    }
}

/// A completed inference call.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmCompletion {
    pub output: Vec<String>,
    pub tokens_used: u64,
    pub model: String,
}

/// A batch of embeddings, one vector per input string.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmEmbeddings {
    pub vectors: Vec<Vec<f64>>,
    pub model: String,
    pub tokens_used: u64,
}

/// OpenAI-compatible driver with live counters.
pub struct LlmDriver {
    client: reqwest::Client,
    config: Mutex<LlmConfig>,
    calls: AtomicU64,
    total_tokens: AtomicU64,
}

impl LlmDriver {
    pub fn new(config: LlmConfig) -> Result<Self, String> {
        if config.base_url.trim().is_empty() {
            return Err("base_url cannot be empty".into());
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("agOS/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(LlmDriver {
            client,
            config: Mutex::new(config),
            calls: AtomicU64::new(0),
            total_tokens: AtomicU64::new(0),
        })
    }

    pub fn config_snapshot(&self) -> LlmConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn redacted_config(&self) -> HashMap<String, String> {
        self.config.lock().unwrap().redacted()
    }

    pub fn update_config(&self, updates: HashMap<String, String>) -> Result<(), String> {
        let mut cfg = self.config.lock().unwrap();
        cfg.apply_updates(&updates)
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }

    /// Send the prompt vector to the provider and return the completion.
    ///
    /// The first element is treated as the system prompt and the rest are
    /// joined into the user message, which matches how the kernel builds
    /// prompt vectors (instructions first, then task data / output format).
    ///
    /// A non-null `schema` switches the provider into JSON mode: `true` or
    /// `{"type":"json_object"}` requests a generic JSON object, while a JSON
    /// Schema (or `{"type":"json_schema","json_schema":{...}}`) requests
    /// structured output. In JSON mode the completion is returned as one
    /// element so downstream code receives parseable JSON instead of lines.
    pub async fn reason(
        &self,
        prompt_vector: &[String],
        schema: &Value,
    ) -> Result<LlmCompletion, String> {
        let cfg = self.config_snapshot();
        let response_format = ResponseFormat::from_schema(schema);
        let json_mode = response_format.is_some();
        let messages = build_messages(prompt_vector, json_mode);
        let prompt_text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let request = ChatRequest {
            model: cfg.model.clone(),
            messages,
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
            response_format,
            stream: false,
            stream_options: None,
        };

        self.calls.fetch_add(1, Ordering::Relaxed);

        let mut req = self
            .client
            .post(cfg.endpoint())
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .json(&request);

        if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.map_err(|e| {
            format!(
                "request to {} failed: {}",
                cfg.endpoint(),
                describe_reqwest_error(&e)
            )
        })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("failed to read model response: {e}"))?;

        if !status.is_success() {
            return Err(format!(
                "model API returned HTTP {}: {}",
                status.as_u16(),
                truncate(&body, MAX_ERROR_BODY)
            ));
        }

        let completion = parse_completion(&body, &prompt_text, json_mode)?;
        self.total_tokens
            .fetch_add(completion.tokens_used, Ordering::Relaxed);
        Ok(completion)
    }

    /// Embed a batch of strings with the configured embedding model.
    ///
    /// Requires `AGOS_LLM_EMBEDDING_MODEL` (or `embedding_model` in the runtime
    /// config). Returns one vector per input, in input order.
    pub async fn embed(&self, data: &[String]) -> Result<LlmEmbeddings, String> {
        let cfg = self.config_snapshot();
        let model = cfg.embedding_model.clone().ok_or_else(|| {
            "embedding model not configured: set AGOS_LLM_EMBEDDING_MODEL or POST \
             /umi/driver/config/OpenAICompatible with {\"embedding_model\": \"...\"}"
                .to_string()
        })?;

        if data.is_empty() {
            return Ok(LlmEmbeddings {
                vectors: Vec::new(),
                model,
                tokens_used: 0,
            });
        }

        let request = EmbeddingRequest {
            model: model.clone(),
            input: data.to_vec(),
            encoding_format: "float",
        };

        self.calls.fetch_add(1, Ordering::Relaxed);

        let mut req = self
            .client
            .post(cfg.embeddings_endpoint())
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .json(&request);

        if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.map_err(|e| {
            format!(
                "request to {} failed: {}",
                cfg.embeddings_endpoint(),
                describe_reqwest_error(&e)
            )
        })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("failed to read embedding response: {e}"))?;

        if !status.is_success() {
            return Err(format!(
                "embedding API returned HTTP {}: {}",
                status.as_u16(),
                truncate(&body, MAX_ERROR_BODY)
            ));
        }

        let embeddings = parse_embeddings(&body, data.len(), &model)?;
        self.total_tokens
            .fetch_add(embeddings.tokens_used, Ordering::Relaxed);
        Ok(embeddings)
    }

    /// Stream a completion, invoking `on_delta` for each text fragment.
    ///
    /// Returning `false` from `on_delta` (for example because the HTTP client
    /// disconnected) drops the provider connection and returns the partial
    /// text with `aborted = "client_closed"`.
    ///
    /// Unlike `reason`, this does not apply a total request timeout: the call
    /// may legitimately run for minutes. Instead it aborts when no bytes arrive
    /// for `options.idle_timeout`, or as soon as the estimated output exceeds
    /// `options.token_budget` (partial text is returned with `aborted` set).
    ///
    /// The return value is authoritative: `text` is the full accumulation,
    /// even after a budget cut-off.
    pub async fn reason_stream<F>(
        &self,
        prompt_vector: &[String],
        schema: &Value,
        options: &StreamOptions,
        mut on_delta: F,
    ) -> Result<LlmStreamResult, String>
    where
        F: FnMut(&str) -> bool + Send,
    {
        let cfg = self.config_snapshot();
        let response_format = ResponseFormat::from_schema(schema);
        let json_mode = response_format.is_some();
        let messages = build_messages(prompt_vector, json_mode);
        let prompt_text = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let request = ChatRequest {
            model: cfg.model.clone(),
            messages,
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
            response_format,
            stream: true,
            stream_options: Some(StreamOptionsRequest {
                include_usage: true,
            }),
        };

        self.calls.fetch_add(1, Ordering::Relaxed);

        let mut req = self.client.post(cfg.endpoint()).json(&request);

        if let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.map_err(|e| {
            format!(
                "request to {} failed: {}",
                cfg.endpoint(),
                describe_reqwest_error(&e)
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .map_err(|e| format!("failed to read model response: {e}"))?;
            return Err(format!(
                "model API returned HTTP {}: {}",
                status.as_u16(),
                truncate(&body, MAX_ERROR_BODY)
            ));
        }

        let mut stream = response.bytes_stream();
        let mut parser = SseParser::default();
        let mut text = String::new();
        let mut provider_tokens: Option<u64> = None;
        let mut model = cfg.model.clone();
        let mut finish_reason: Option<String> = None;
        let mut aborted: Option<String> = None;
        let mut chunks = 0usize;
        let mut finished = false;

        'stream: while !finished {
            let next = tokio::time::timeout(options.idle_timeout, stream.next()).await;
            let bytes = match next {
                Err(_) => {
                    return Err(format!(
                        "model stream stalled: no data for {:?}",
                        options.idle_timeout
                    ))
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(format!("model stream error: {e}")),
                Ok(Some(Ok(bytes))) => bytes,
            };

            for payload in parser.push(&bytes) {
                if payload == "[DONE]" {
                    finished = true;
                    break;
                }

                let parsed: StreamChunk = match serde_json::from_str(&payload) {
                    Ok(chunk) => chunk,
                    // Ignore keepalives / non-JSON provider noise.
                    Err(_) => continue,
                };

                if let Some(error) = parsed.error {
                    return Err(format!(
                        "model stream error: {}",
                        error
                            .message
                            .or(error.kind)
                            .unwrap_or_else(|| "unknown provider error".to_string())
                    ));
                }

                if let Some(name) = parsed.model {
                    model = name;
                }
                if let Some(tokens) = parsed.usage.and_then(|usage| usage.total_tokens) {
                    provider_tokens = Some(tokens);
                }

                if let Some(choice) = parsed.choices.into_iter().next() {
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                    if let Some(delta) = choice.delta.content {
                        if !delta.is_empty() {
                            text.push_str(&delta);
                            chunks += 1;
                            if !on_delta(&delta) {
                                aborted = Some("client_closed".to_string());
                                break 'stream;
                            }
                        }
                    }
                }

                if let Some(budget) = options.token_budget {
                    if estimate_tokens(&text) > budget {
                        aborted = Some("token_budget".to_string());
                        break 'stream;
                    }
                }
            }
        }

        let tokens_used = provider_tokens
            .filter(|tokens| *tokens > 0)
            .unwrap_or_else(|| estimate_tokens(&prompt_text) + estimate_tokens(&text));
        self.total_tokens.fetch_add(tokens_used, Ordering::Relaxed);

        let output = split_response(&text, json_mode);
        Ok(LlmStreamResult {
            output,
            text,
            tokens_used,
            model,
            finish_reason,
            aborted,
            chunks,
        })
    }
}

/// Controls for `reason_stream`.
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Abort the call when the provider sends nothing for this long.
    pub idle_timeout: Duration,
    /// Stop reading once the accumulated output estimate exceeds this.
    pub token_budget: Option<u64>,
}

impl Default for StreamOptions {
    fn default() -> Self {
        StreamOptions {
            idle_timeout: Duration::from_secs(DEFAULT_STREAM_IDLE_TIMEOUT_SECS),
            token_budget: None,
        }
    }
}

/// Outcome of a streamed completion.
#[derive(Debug, Clone)]
pub struct LlmStreamResult {
    /// Full accumulated text, including any partial output after a cut-off.
    pub text: String,
    /// Line-oriented view used by the buffered API (unsplit in JSON mode).
    pub output: Vec<String>,
    pub tokens_used: u64,
    pub model: String,
    pub finish_reason: Option<String>,
    /// Set when the kernel stopped early (`token_budget`).
    pub aborted: Option<String>,
    /// Number of non-empty content fragments received.
    pub chunks: usize,
}

/// Minimal server-sent-events reassembler.
///
/// Providers may split an event across TCP chunks, use `\r\n` line endings, or
/// send multi-line `data:` fields. Bytes are buffered until a blank line marks
/// the end of an event.
#[derive(Debug, Default)]
struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some((end, sep_len)) = find_event_boundary(&self.buffer) {
            let block = String::from_utf8_lossy(&self.buffer[..end]).into_owned();
            self.buffer.drain(..end + sep_len);
            if let Some(payload) = parse_sse_block(&block) {
                events.push(payload);
            }
        }
        events
    }
}

fn find_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, 2));
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn parse_sse_block(block: &str) -> Option<String> {
    let mut data = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
    }
    if data.is_empty() {
        None
    } else {
        Some(data.join("\n"))
    }
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<StreamError>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamError {
    #[serde(default)]
    message: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptionsRequest>,
}

/// Ask the provider to include a final `usage` chunk while streaming.
#[derive(Debug, Serialize)]
struct StreamOptionsRequest {
    include_usage: bool,
}

/// Provider-side structured-output selector.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    JsonObject,
    JsonSchema { json_schema: JsonSchemaSpec },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JsonSchemaSpec {
    pub name: String,
    pub schema: Value,
    #[serde(default)]
    pub strict: bool,
}

impl ResponseFormat {
    /// Translate the kernel's `schema` argument into a provider hint.
    ///
    /// Accepted forms:
    /// - `true` or `"json"` / `"json_object"` → `{"type":"json_object"}`
    /// - `{"type":"json_object"}` → `{"type":"json_object"}`
    /// - any JSON Schema (`{"type":"object","properties":{...}}`) → strict-less
    ///   `json_schema` wrapper
    /// - `{"type":"json_schema","json_schema":{...}}` → passed through
    ///
    /// Anything else is ignored so callers can pass arbitrary `schema` values
    /// (the orchestrator uses `{"task": ...}` as a hint) without failing.
    pub fn from_schema(schema: &Value) -> Option<Self> {
        match schema {
            Value::Null => None,
            Value::Bool(true) => Some(ResponseFormat::JsonObject),
            Value::String(value) => {
                if is_generic_json(value) {
                    Some(ResponseFormat::JsonObject)
                } else {
                    None
                }
            }
            Value::Object(map) => {
                if let Some(inner) = map.get("json_schema") {
                    let schema = inner.get("schema")?.clone();
                    return Some(ResponseFormat::JsonSchema {
                        json_schema: JsonSchemaSpec {
                            name: inner
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("agos_response")
                                .to_string(),
                            schema,
                            strict: inner
                                .get("strict")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        },
                    });
                }

                if let Some(kind) = map.get("type").and_then(Value::as_str) {
                    if is_generic_json(kind) {
                        return Some(ResponseFormat::JsonObject);
                    }
                    if kind == "json_schema" {
                        let schema = map.get("schema")?.clone();
                        return Some(ResponseFormat::JsonSchema {
                            json_schema: JsonSchemaSpec {
                                name: map
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("agos_response")
                                    .to_string(),
                                schema,
                                strict: map.get("strict").and_then(Value::as_bool).unwrap_or(false),
                            },
                        });
                    }
                }

                let looks_like_schema = map.contains_key("properties")
                    || map.contains_key("$schema")
                    || map.contains_key("required")
                    || map
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(is_json_schema_type);

                if looks_like_schema {
                    return Some(ResponseFormat::JsonSchema {
                        json_schema: JsonSchemaSpec {
                            name: map
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("agos_response")
                                .to_string(),
                            schema: schema.clone(),
                            strict: map.get("strict").and_then(Value::as_bool).unwrap_or(false),
                        },
                    });
                }

                None
            }
            _ => None,
        }
    }
}

fn is_generic_json(value: &str) -> bool {
    value.eq_ignore_ascii_case("json") || value.eq_ignore_ascii_case("json_object")
}

fn is_json_schema_type(value: &str) -> bool {
    matches!(
        value,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    )
}

#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
    encoding_format: &'static str,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<EmbeddingItem>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f64>,
    #[serde(default)]
    index: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
    #[serde(default)]
    total_tokens: Option<u64>,
}

/// Map the kernel's prompt vector onto chat messages.
///
/// When `json_mode` is set, OpenAI requires the word "json" to appear in the
/// messages, so a short instruction is appended if the caller did not mention
/// it already.
pub fn build_messages(prompt_vector: &[String], json_mode: bool) -> Vec<ChatMessage> {
    let mut messages = match prompt_vector {
        [] => vec![ChatMessage {
            role: "user".into(),
            content: String::new(),
        }],
        [single] => vec![ChatMessage {
            role: "user".into(),
            content: single.clone(),
        }],
        [system, rest @ ..] => {
            let user = rest.join("\n\n");
            if system.trim().is_empty() {
                vec![ChatMessage {
                    role: "user".into(),
                    content: user,
                }]
            } else {
                vec![
                    ChatMessage {
                        role: "system".into(),
                        content: system.clone(),
                    },
                    ChatMessage {
                        role: "user".into(),
                        content: user,
                    },
                ]
            }
        }
    };

    if json_mode {
        let mentions_json = messages
            .iter()
            .any(|m| m.content.to_ascii_lowercase().contains("json"));
        if !mentions_json {
            const HINT: &str = "Respond with a single JSON object and no surrounding prose.";
            match messages.first_mut() {
                Some(first) if first.role == "system" => {
                    first.content.push_str("\n\n");
                    first.content.push_str(HINT);
                }
                _ => messages.insert(
                    0,
                    ChatMessage {
                        role: "system".into(),
                        content: HINT.to_string(),
                    },
                ),
            }
        }
    }

    messages
}

/// Parse a Chat Completions response body.
fn parse_completion(
    body: &str,
    prompt_text: &str,
    json_mode: bool,
) -> Result<LlmCompletion, String> {
    let parsed: ChatResponse =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON from model API: {e}"))?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "model API returned no choices".to_string())?;

    let content = choice.message.content.unwrap_or_default();
    let output = split_response(&content, json_mode);

    let tokens_from_api = parsed.usage.and_then(|usage| {
        usage
            .total_tokens
            .or_else(|| match (usage.prompt_tokens, usage.completion_tokens) {
                (Some(prompt), Some(completion)) => Some(prompt + completion),
                (Some(prompt), None) => Some(prompt),
                (None, Some(completion)) => Some(completion),
                (None, None) => None,
            })
    });

    let tokens_used = tokens_from_api
        .filter(|tokens| *tokens > 0)
        .unwrap_or_else(|| estimate_tokens(prompt_text) + estimate_tokens(&content));

    Ok(LlmCompletion {
        output,
        tokens_used,
        model: parsed.model.unwrap_or_default(),
    })
}

/// Parse an embeddings response, preserving input order.
fn parse_embeddings(
    body: &str,
    expected: usize,
    fallback_model: &str,
) -> Result<LlmEmbeddings, String> {
    let mut parsed: EmbeddingResponse =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON from embedding API: {e}"))?;

    if parsed.data.len() != expected {
        return Err(format!(
            "embedding API returned {} vectors for {} inputs",
            parsed.data.len(),
            expected
        ));
    }

    if parsed.data.iter().all(|item| item.index.is_some()) {
        parsed.data.sort_by_key(|item| item.index.unwrap_or(0));
    }

    let dimension = parsed
        .data
        .first()
        .map(|item| item.embedding.len())
        .unwrap_or(0);
    if dimension == 0 {
        return Err("embedding API returned empty vectors".to_string());
    }
    if let Some(bad) = parsed
        .data
        .iter()
        .find(|item| item.embedding.len() != dimension)
    {
        return Err(format!(
            "embedding API returned inconsistent dimensions: {} and {}",
            dimension,
            bad.embedding.len()
        ));
    }

    let tokens_used = parsed
        .usage
        .and_then(|usage| usage.total_tokens.or(usage.prompt_tokens))
        .unwrap_or(0);

    Ok(LlmEmbeddings {
        vectors: parsed.data.into_iter().map(|item| item.embedding).collect(),
        model: parsed.model.unwrap_or_else(|| fallback_model.to_string()),
        tokens_used,
    })
}

/// Split a completion into lines because downstream consumers parse per line.
///
/// In JSON mode the payload must stay intact, so it is returned as one element.
fn split_response(text: &str, json_mode: bool) -> Vec<String> {
    if json_mode {
        let trimmed = text.trim();
        return if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_string()]
        };
    }

    text.lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Rough token estimate used only when a provider omits `usage`.
fn estimate_tokens(text: &str) -> u64 {
    ((text.chars().count() as f64) / 4.0).ceil() as u64
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &text[..end])
    }
}

fn describe_reqwest_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "request timed out".to_string()
    } else if error.is_connect() {
        "could not connect to the model endpoint".to_string()
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use std::sync::Arc;

    fn cfg_for(addr: std::net::SocketAddr) -> LlmConfig {
        LlmConfig::new(format!("http://{addr}/v1"))
            .with_model("test-model")
            .with_timeout_secs(5)
    }

    #[test]
    fn maps_single_prompt_to_user_message() {
        let messages = build_messages(&["Say hello".to_string()], false);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Say hello");
    }

    #[test]
    fn maps_prompt_vector_to_system_and_user() {
        let messages = build_messages(
            &[
                "You are an auditor.".to_string(),
                "Invoice A: 100".to_string(),
                "Reply in JSON.".to_string(),
            ],
            false,
        );
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, "You are an auditor.");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "Invoice A: 100\n\nReply in JSON.");
    }

    #[test]
    fn maps_empty_prompt_to_empty_user_message() {
        let messages = build_messages(&[], false);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "");
    }

    #[test]
    fn parses_usage_from_response() {
        let body = json!({
            "model": "gpt-4o-mini",
            "choices": [{"message": {"role": "assistant", "content": "RESOLVED: x\nAMBIGUOUS: y"}}],
            "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
        })
        .to_string();
        let completion = parse_completion(&body, "prompt", false).unwrap();
        assert_eq!(completion.output, vec!["RESOLVED: x", "AMBIGUOUS: y"]);
        assert_eq!(completion.tokens_used, 18);
        assert_eq!(completion.model, "gpt-4o-mini");
    }

    #[test]
    fn estimates_usage_when_provider_omits_it() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "12345678"}}]
        })
        .to_string();
        let completion = parse_completion(&body, "12345678", false).unwrap();
        assert_eq!(completion.tokens_used, 4);
    }

    #[test]
    fn rejects_response_without_choices() {
        let body = json!({"error": {"message": "nope"}}).to_string();
        let err = parse_completion(&body, "", false).unwrap_err();
        assert!(err.contains("no choices"), "{err}");
    }

    #[test]
    fn validates_config_updates() {
        let mut cfg = LlmConfig::new("https://example.test/v1");
        let mut updates = HashMap::new();
        updates.insert("model".to_string(), "deepseek-chat".to_string());
        updates.insert("timeout_secs".to_string(), "15".to_string());
        cfg.apply_updates(&updates).unwrap();
        assert_eq!(cfg.model, "deepseek-chat");
        assert_eq!(cfg.timeout_secs, 15);

        let mut bad = HashMap::new();
        bad.insert("timeout_secs".to_string(), "0".to_string());
        assert!(cfg.apply_updates(&bad).is_err());

        let mut unknown = HashMap::new();
        unknown.insert("temperature_x".to_string(), "1".to_string());
        assert!(cfg.apply_updates(&unknown).is_err());
    }

    #[test]
    fn redacts_api_key() {
        let cfg = LlmConfig::new("https://example.test/v1").with_api_key("sk-secret");
        let redacted = cfg.redacted();
        assert_eq!(redacted.get("api_key").map(String::as_str), Some("***"));
        assert!(!format!("{redacted:?}").contains("sk-secret"));
    }

    #[derive(Clone)]
    struct MockState {
        received: Arc<Mutex<Vec<Value>>>,
        status: u16,
    }

    async fn mock_completions(
        State(state): State<MockState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (axum::http::StatusCode, Json<Value>) {
        let mut received = state.received.lock().unwrap();
        received.push(json!({
            "body": body,
            "authorization": headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or(""),
        }));

        if state.status != 200 {
            return (
                axum::http::StatusCode::from_u16(state.status).unwrap(),
                Json(json!({"error": {"message": "insufficient_quota"}})),
            );
        }

        (
            axum::http::StatusCode::OK,
            Json(json!({
                "model": "test-model-0613",
                "choices": [{"message": {"role": "assistant", "content": "hello\nworld"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            })),
        )
    }

    async fn mock_embeddings(
        State(state): State<MockState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (axum::http::StatusCode, Json<Value>) {
        let mut received = state.received.lock().unwrap();
        received.push(json!({
            "body": body,
            "authorization": headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or(""),
        }));

        let inputs = body["input"].as_array().cloned().unwrap_or_default();
        let data: Vec<Value> = inputs
            .iter()
            .enumerate()
            .map(|(i, _)| {
                json!({
                    "index": i,
                    "embedding": [0.1 * (i as f64 + 1.0), 0.25, 0.5]
                })
            })
            .collect();

        (
            axum::http::StatusCode::OK,
            Json(json!({
                "model": "text-embedding-test",
                "data": data,
                "usage": {"prompt_tokens": 4, "total_tokens": 4}
            })),
        )
    }

    #[derive(Clone)]
    struct StreamMockState {
        received: Arc<Mutex<Vec<Value>>>,
        chunks: Arc<Vec<String>>,
        stall: bool,
    }

    async fn mock_stream(
        State(state): State<StreamMockState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        state.received.lock().unwrap().push(json!({
            "body": body,
            "authorization": headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or(""),
        }));

        let chunks = state
            .chunks
            .as_ref()
            .clone()
            .into_iter()
            .map(|chunk| Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(chunk)));
        let stream = futures_util::stream::iter(chunks);
        let body = if state.stall {
            axum::body::Body::from_stream(stream.chain(futures_util::stream::pending()))
        } else {
            axum::body::Body::from_stream(stream)
        };

        axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(body)
            .unwrap()
    }

    async fn spawn_stream_mock(
        chunks: Vec<&'static str>,
        stall: bool,
    ) -> (std::net::SocketAddr, Arc<Mutex<Vec<Value>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = StreamMockState {
            received: received.clone(),
            chunks: Arc::new(chunks.into_iter().map(str::to_string).collect()),
            stall,
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_stream))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, received)
    }

    async fn spawn_mock(status: u16) -> (std::net::SocketAddr, Arc<Mutex<Vec<Value>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            received: received.clone(),
            status,
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_completions))
            .route("/v1/embeddings", post(mock_embeddings))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, received)
    }

    #[tokio::test]
    async fn calls_an_openai_compatible_endpoint() {
        let (addr, received) = spawn_mock(200).await;
        let driver = LlmDriver::new(cfg_for(addr).with_api_key("sk-test")).unwrap();

        let completion = driver
            .reason(
                &["You are terse.".to_string(), "Greet me.".to_string()],
                &Value::Null,
            )
            .await
            .unwrap();

        assert_eq!(completion.output, vec!["hello", "world"]);
        assert_eq!(completion.tokens_used, 7);
        assert_eq!(completion.model, "test-model-0613");
        assert_eq!(driver.calls(), 1);
        assert_eq!(driver.total_tokens(), 7);

        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["authorization"], "Bearer sk-test");
        assert_eq!(received[0]["body"]["model"], "test-model");
        assert_eq!(received[0]["body"]["messages"][0]["role"], "system");
        assert_eq!(received[0]["body"]["messages"][1]["content"], "Greet me.");
    }

    #[tokio::test]
    async fn surfaces_provider_errors() {
        let (addr, _) = spawn_mock(401).await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let err = driver
            .reason(&["hello".to_string()], &Value::Null)
            .await
            .unwrap_err();

        assert!(err.contains("401"), "{err}");
        assert!(err.contains("insufficient_quota"), "{err}");
    }

    #[test]
    fn json_mode_appends_hint_when_missing() {
        let messages = build_messages(&["You are an auditor.".to_string()], true);
        assert_eq!(messages[0].role, "system");
        assert!(messages[0].content.to_ascii_lowercase().contains("json"));
        assert_eq!(messages[1].role, "user");
        assert!(messages[1].content.contains("You are an auditor."));
    }

    #[test]
    fn json_mode_does_not_duplicate_existing_hint() {
        let messages = build_messages(&["Reply in JSON.".to_string()], true);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Reply in JSON.");
    }

    #[test]
    fn maps_generic_json_requests_to_json_object() {
        for schema in [
            json!(true),
            json!("json"),
            json!("json_object"),
            json!({"type": "json_object"}),
        ] {
            assert_eq!(
                ResponseFormat::from_schema(&schema),
                Some(ResponseFormat::JsonObject),
                "schema: {schema}"
            );
        }
    }

    #[test]
    fn maps_json_schema_documents() {
        let schema = json!({
            "type": "object",
            "name": "invoice",
            "properties": {"total": {"type": "number"}},
            "required": ["total"]
        });
        let format = ResponseFormat::from_schema(&schema).unwrap();
        let serialized = serde_json::to_value(&format).unwrap();
        assert_eq!(serialized["type"], "json_schema");
        assert_eq!(serialized["json_schema"]["name"], "invoice");
        assert_eq!(serialized["json_schema"]["strict"], false);
        assert_eq!(
            serialized["json_schema"]["schema"]["properties"]["total"]["type"],
            "number"
        );
    }

    #[test]
    fn passes_through_wrapped_json_schema() {
        let schema = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "answer",
                "strict": true,
                "schema": {"type": "object", "properties": {"ok": {"type": "boolean"}}}
            }
        });
        let format = ResponseFormat::from_schema(&schema).unwrap();
        let serialized = serde_json::to_value(&format).unwrap();
        assert_eq!(serialized["type"], "json_schema");
        assert_eq!(serialized["json_schema"]["name"], "answer");
        assert_eq!(serialized["json_schema"]["strict"], true);
    }

    #[test]
    fn ignores_unrelated_hint_objects() {
        assert_eq!(
            ResponseFormat::from_schema(&json!({"task": "condition_check"})),
            None
        );
        assert_eq!(ResponseFormat::from_schema(&Value::Null), None);
        assert_eq!(ResponseFormat::from_schema(&json!("yaml")), None);
    }

    #[test]
    fn json_mode_keeps_multiline_payload_intact() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "{\n  \"ok\": true\n}"}}]
        })
        .to_string();
        let completion = parse_completion(&body, "prompt", true).unwrap();
        assert_eq!(completion.output, vec!["{\n  \"ok\": true\n}"]);
    }

    #[test]
    fn parses_embeddings_in_index_order() {
        let body = json!({
            "model": "text-embedding-test",
            "data": [
                {"index": 1, "embedding": [0.2, 0.2]},
                {"index": 0, "embedding": [0.1, 0.1]}
            ],
            "usage": {"prompt_tokens": 6}
        })
        .to_string();
        let embeddings = parse_embeddings(&body, 2, "fallback").unwrap();
        assert_eq!(embeddings.vectors, vec![vec![0.1, 0.1], vec![0.2, 0.2]]);
        assert_eq!(embeddings.tokens_used, 6);
        assert_eq!(embeddings.model, "text-embedding-test");
    }

    #[test]
    fn rejects_mismatched_embedding_count() {
        let body = json!({"data": [{"index": 0, "embedding": [0.1]}]}).to_string();
        let err = parse_embeddings(&body, 2, "m").unwrap_err();
        assert!(err.contains("1 vectors for 2 inputs"), "{err}");
    }

    #[tokio::test]
    async fn sends_response_format_when_schema_is_given() {
        let (addr, received) = spawn_mock(200).await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let completion = driver
            .reason(&["Summarize.".to_string()], &json!({"type": "json_object"}))
            .await
            .unwrap();

        // JSON payloads are not split into lines.
        assert_eq!(completion.output, vec!["hello\nworld"]);

        let received = received.lock().unwrap();
        assert_eq!(
            received[0]["body"]["response_format"]["type"],
            "json_object"
        );
        assert!(received[0]["body"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("json"));
    }

    #[tokio::test]
    async fn embeds_with_the_configured_model() {
        let (addr, received) = spawn_mock(200).await;
        let driver =
            LlmDriver::new(cfg_for(addr).with_embedding_model("text-embedding-test")).unwrap();

        let embeddings = driver
            .embed(&["alpha".to_string(), "beta".to_string()])
            .await
            .unwrap();

        assert_eq!(embeddings.vectors.len(), 2);
        assert_eq!(embeddings.vectors[0], vec![0.1, 0.25, 0.5]);
        assert_eq!(embeddings.vectors[1], vec![0.2, 0.25, 0.5]);
        assert_eq!(embeddings.model, "text-embedding-test");
        assert_eq!(embeddings.tokens_used, 4);
        assert_eq!(driver.total_tokens(), 4);

        let received = received.lock().unwrap();
        assert_eq!(received[0]["body"]["model"], "text-embedding-test");
        assert_eq!(received[0]["body"]["input"][1], "beta");
    }

    #[tokio::test]
    async fn embedding_requires_a_model() {
        let (addr, _) = spawn_mock(200).await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let err = driver.embed(&["alpha".to_string()]).await.unwrap_err();
        assert!(err.contains("embedding model not configured"), "{err}");
    }

    #[test]
    fn sse_parser_reassembles_split_events() {
        let mut parser = SseParser::default();
        assert!(parser.push(b"data: one\n").is_empty());
        assert_eq!(
            parser.push(b"\ndata: two\r\n\r\ndata: thr"),
            vec!["one".to_string(), "two".to_string()]
        );
        assert_eq!(parser.push(b"ee\n\n"), vec!["three".to_string()]);
    }

    #[test]
    fn sse_parser_joins_multiline_data() {
        let mut parser = SseParser::default();
        assert_eq!(
            parser.push(b"data: {\"a\":\ndata: 1}\n\n"),
            vec!["{\"a\":\n1}".to_string()]
        );
    }

    fn stream_chunks() -> Vec<&'static str> {
        vec![
            "data: {\"model\":\"stream-model\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        ]
    }

    #[tokio::test]
    async fn streams_deltas_and_final_usage() {
        let (addr, received) = spawn_stream_mock(stream_chunks(), false).await;
        let driver = LlmDriver::new(cfg_for(addr).with_api_key("sk-stream")).unwrap();

        let mut deltas = Vec::new();
        let result = driver
            .reason_stream(
                &["hi".to_string()],
                &Value::Null,
                &StreamOptions {
                    idle_timeout: Duration::from_secs(5),
                    token_budget: None,
                },
                |delta| {
                    deltas.push(delta.to_string());
                    true
                },
            )
            .await
            .unwrap();

        assert_eq!(deltas, vec!["Hel", "lo"]);
        assert_eq!(result.text, "Hello");
        assert_eq!(result.output, vec!["Hello"]);
        assert_eq!(result.tokens_used, 5);
        assert_eq!(result.model, "stream-model");
        assert_eq!(result.finish_reason.as_deref(), Some("stop"));
        assert_eq!(result.chunks, 2);
        assert!(result.aborted.is_none());
        assert_eq!(driver.total_tokens(), 5);

        let received = received.lock().unwrap();
        assert_eq!(received[0]["authorization"], "Bearer sk-stream");
        assert_eq!(received[0]["body"]["stream"], true);
        assert_eq!(received[0]["body"]["stream_options"]["include_usage"], true);
    }

    #[tokio::test]
    async fn stream_stops_at_the_token_budget() {
        let (addr, _) = spawn_stream_mock(stream_chunks(), false).await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let result = driver
            .reason_stream(
                &["hi".to_string()],
                &Value::Null,
                &StreamOptions {
                    idle_timeout: Duration::from_secs(5),
                    token_budget: Some(0),
                },
                |_| true,
            )
            .await
            .unwrap();

        assert_eq!(result.aborted.as_deref(), Some("token_budget"));
        assert_eq!(result.text, "Hel");
        assert_eq!(result.chunks, 1);
    }

    #[tokio::test]
    async fn stream_times_out_when_provider_stalls() {
        let (addr, _) = spawn_stream_mock(
            vec!["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n"],
            true,
        )
        .await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let err = driver
            .reason_stream(
                &["hi".to_string()],
                &Value::Null,
                &StreamOptions {
                    idle_timeout: Duration::from_millis(200),
                    token_budget: None,
                },
                |_| true,
            )
            .await
            .unwrap_err();

        assert!(err.contains("stalled"), "{err}");
    }

    #[tokio::test]
    async fn stream_surfaces_provider_error_events() {
        let (addr, _) = spawn_stream_mock(
            vec!["data: {\"error\":{\"message\":\"boom\",\"type\":\"server_error\"}}\n\n"],
            false,
        )
        .await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let err = driver
            .reason_stream(
                &["hi".to_string()],
                &Value::Null,
                &StreamOptions::default(),
                |_| true,
            )
            .await
            .unwrap_err();

        assert!(err.contains("boom"), "{err}");
    }

    #[tokio::test]
    async fn stream_stops_when_the_client_disconnects() {
        let (addr, _) = spawn_stream_mock(stream_chunks(), false).await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let result = driver
            .reason_stream(
                &["hi".to_string()],
                &Value::Null,
                &StreamOptions::default(),
                |_| false,
            )
            .await
            .unwrap();

        assert_eq!(result.aborted.as_deref(), Some("client_closed"));
        assert_eq!(result.text, "Hel");
        assert_eq!(result.chunks, 1);
    }

    #[tokio::test]
    async fn stream_keeps_json_payload_unsplit() {
        let (addr, _) = spawn_stream_mock(
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"a\\\":\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"1}\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ],
            false,
        )
        .await;
        let driver = LlmDriver::new(cfg_for(addr)).unwrap();

        let result = driver
            .reason_stream(
                &["hi".to_string()],
                &json!({"type": "json_object"}),
                &StreamOptions::default(),
                |_| true,
            )
            .await
            .unwrap();

        assert_eq!(result.text, "{\"a\":1}");
        assert_eq!(result.output, vec!["{\"a\":1}"]);
    }
}
