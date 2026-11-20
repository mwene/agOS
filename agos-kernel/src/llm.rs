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
//! | `AGOS_LLM_TIMEOUT_SECS` | Per-request timeout | `60` |
//! | `AGOS_LLM_MAX_TOKENS` | `max_tokens` sent to the provider | provider default |
//! | `AGOS_LLM_TEMPERATURE` | Sampling temperature | provider default |
//!
//! `OPENAI_BASE_URL` / `OPENAI_API_KEY` are accepted as fallbacks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Name a driver is registered under inside the UMI.
pub const DRIVER_NAME: &str = "OpenAICompatible";

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_ERROR_BODY: usize = 500;

/// Configuration for an OpenAI-compatible endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub timeout_secs: u64,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl LlmConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        LlmConfig {
            base_url: base_url.into(),
            api_key: None,
            model: DEFAULT_MODEL.to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
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
        let timeout_secs = env_non_empty("AGOS_LLM_TIMEOUT_SECS")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let max_tokens = env_non_empty("AGOS_LLM_MAX_TOKENS").and_then(|v| v.parse().ok());
        let temperature = env_non_empty("AGOS_LLM_TEMPERATURE").and_then(|v| v.parse().ok());

        Some(LlmConfig {
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            api_key,
            model,
            timeout_secs,
            max_tokens,
            temperature,
        })
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
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

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Full URL of the Chat Completions endpoint.
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Configuration safe to return over the API (the key is masked).
    pub fn redacted(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("base_url".into(), self.base_url.clone());
        map.insert("model".into(), self.model.clone());
        map.insert("timeout_secs".into(), self.timeout_secs.to_string());
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
                "timeout_secs" => {
                    let secs: u64 = value.parse().map_err(|_| {
                        format!("timeout_secs must be a positive integer, got '{value}'")
                    })?;
                    if secs == 0 {
                        return Err("timeout_secs must be greater than zero".into());
                    }
                    next.timeout_secs = secs;
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
    pub async fn reason(&self, prompt_vector: &[String]) -> Result<LlmCompletion, String> {
        let cfg = self.config_snapshot();
        let messages = build_messages(prompt_vector);
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
            stream: false,
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

        let completion = parse_completion(&body, &prompt_text)?;
        self.total_tokens
            .fetch_add(completion.tokens_used, Ordering::Relaxed);
        Ok(completion)
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
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
pub fn build_messages(prompt_vector: &[String]) -> Vec<ChatMessage> {
    match prompt_vector {
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
    }
}

/// Parse a Chat Completions response body.
fn parse_completion(body: &str, prompt_text: &str) -> Result<LlmCompletion, String> {
    let parsed: ChatResponse =
        serde_json::from_str(body).map_err(|e| format!("invalid JSON from model API: {e}"))?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "model API returned no choices".to_string())?;

    let content = choice.message.content.unwrap_or_default();
    let output = split_response(&content);

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

/// Split a completion into lines because downstream consumers parse per line.
fn split_response(text: &str) -> Vec<String> {
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
        let messages = build_messages(&["Say hello".to_string()]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Say hello");
    }

    #[test]
    fn maps_prompt_vector_to_system_and_user() {
        let messages = build_messages(&[
            "You are an auditor.".to_string(),
            "Invoice A: 100".to_string(),
            "Reply in JSON.".to_string(),
        ]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, "You are an auditor.");
        assert_eq!(messages[1].role, "user");
        assert_eq!(messages[1].content, "Invoice A: 100\n\nReply in JSON.");
    }

    #[test]
    fn maps_empty_prompt_to_empty_user_message() {
        let messages = build_messages(&[]);
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
        let completion = parse_completion(&body, "prompt").unwrap();
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
        let completion = parse_completion(&body, "12345678").unwrap();
        assert_eq!(completion.tokens_used, 4);
    }

    #[test]
    fn rejects_response_without_choices() {
        let body = json!({"error": {"message": "nope"}}).to_string();
        let err = parse_completion(&body, "").unwrap_err();
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

    async fn spawn_mock(status: u16) -> (std::net::SocketAddr, Arc<Mutex<Vec<Value>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            received: received.clone(),
            status,
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(mock_completions))
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
            .reason(&["You are terse.".to_string(), "Greet me.".to_string()])
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

        let err = driver.reason(&["hello".to_string()]).await.unwrap_err();

        assert!(err.contains("401"), "{err}");
        assert!(err.contains("insufficient_quota"), "{err}");
    }
}
