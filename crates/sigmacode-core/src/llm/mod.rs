use async_trait::async_trait;
use futures::{Stream, StreamExt};
use std::pin::Pin;

use crate::error::Result;
use crate::rate_limit::{LlmRateLimiter, RateLimitResult};
use crate::security::SecurityGuard;
use crate::types::{CompletionOptions, CompletionResponse, LlmEvent, Message, TokenUsage, ProviderConfig};

// ── LLM Provider Trait ──

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse>;

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent>> + Send>>>;
}

// ── Error Parsing ──

fn parse_error_message(text: &str) -> String {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(msg) = json["error"]["message"].as_str() {
            return msg.to_string();
        }
        if let Some(msg) = json["message"].as_str() {
            return msg.to_string();
        }
        if let Some(msg) = json["error"].as_str() {
            return msg.to_string();
        }
    }
    let clean = text.trim();
    if clean.len() > 200 {
        format!("{}...", &clean[..200])
    } else {
        clean.to_string()
    }
}

// ── OpenAI-Compatible Provider ──

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    security: SecurityGuard,
    rate_limiter: LlmRateLimiter,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            api_key,
            base_url,
            model,
            security: SecurityGuard::new(),
            rate_limiter: LlmRateLimiter::new(),
        }
    }

    pub fn with_security(mut self, security: SecurityGuard) -> Self {
        self.security = security;
        self
    }

    pub fn with_rate_limiter(mut self, rate_limiter: LlmRateLimiter) -> Self {
        self.rate_limiter = rate_limiter;
        self
    }

    fn build_request_body(
        &self,
        messages: &[Message],
        _tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
        stream: bool,
    ) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                match m {
                    Message::Assistant { content, tool_calls } if !tool_calls.is_empty() => {
                        let mut text = content.clone().unwrap_or_default();
                        for tc in tool_calls {
                            let args_str = if let Ok(s) = serde_json::to_string(&tc.arguments) { s } else { "{}".into() };
                            text.push_str(&format!(
                                "\n```tool_call\n{{\"tool\": \"{}\", \"args\": {}}}\n```",
                                tc.name, args_str
                            ));
                        }
                        serde_json::json!({
                            "role": "assistant",
                            "content": text,
                        })
                    }
                    Message::Tool { tool_call_id, content } => {
                        serde_json::json!({
                            "role": "user",
                            "content": format!("[Tool result: {}]\n{}", tool_call_id, content),
                        })
                    }
                    _ => {
                        serde_json::to_value(m).unwrap_or_default()
                    }
                }
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": options.max_tokens.unwrap_or(4096),
            "temperature": options.temperature.unwrap_or(0.0),
        });

        if stream {
            body["stream"] = serde_json::json!(true);
        }

        body
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("openai").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let body = self.build_request_body(messages, tools, options, false);

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            if status == 429 {
                return Err(crate::error::SigmaError::RateLimited { retry_after_ms: 5000 });
            }
            if status == 401 || status == 403 {
                return Err(crate::error::SigmaError::LlmAuth(msg));
            }
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let json: serde_json::Value = response.json().await?;

        let choice = &json["choices"][0];
        let message = &choice["message"];

        let content = message["content"].as_str().map(|s| s.to_string());

        let tool_calls = message["tool"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        Some(crate::types::ToolCall {
                            id: tc["id"].as_str()?.to_string(),
                            name: tc["function"]["name"].as_str()?.to_string(),
                            arguments: serde_json::from_str(
                                tc["function"]["arguments"].as_str().unwrap_or("{}"),
                            )
                            .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let usage = TokenUsage {
            prompt_tokens: json["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: json["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResponse {
            content,
            tool_calls,
            usage,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent>> + Send>>> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("openai").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let body = self.build_request_body(messages, tools, options, true);

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            if status == 429 {
                return Err(crate::error::SigmaError::RateLimited { retry_after_ms: 5000 });
            }
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let stream = response.bytes_stream();
        let event_stream = futures::StreamExt::flat_map(stream, |chunk| {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    return futures::stream::iter(vec![Err(crate::error::SigmaError::Llm(e.to_string()))]).boxed()
                }
            };
            let text = String::from_utf8_lossy(&chunk).to_string();
            let mut events = Vec::new();

            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line == "data: [DONE]" {
                    events.push(Ok(LlmEvent::Done {
                        usage: TokenUsage::default(),
                    }));
                    break;
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = json["choices"][0]["delta"].as_object() {
                            if let Some(content) = delta.get("content") {
                                if let Some(text) = content.as_str() {
                                    events.push(Ok(LlmEvent::ContentDelta(text.to_string())));
                                }
                            }
                        }
                        if let Some(usage) = json.get("usage") {
                            events.push(Ok(LlmEvent::Done {
                                usage: TokenUsage {
                                    prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                                    completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0) as u32,
                                    total_tokens: usage["total_tokens"].as_u64().unwrap_or(0) as u32,
                                },
                            }));
                        }
                    }
                }
            }
            futures::stream::iter(events).boxed()
        });

        Ok(Box::pin(event_stream))
    }
}

// ── Anthropic Provider ──

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    security: SecurityGuard,
    rate_limiter: LlmRateLimiter,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            security: SecurityGuard::new(),
            rate_limiter: LlmRateLimiter::new(),
        }
    }

    pub fn with_security(mut self, security: SecurityGuard) -> Self {
        self.security = security;
        self
    }

    pub fn with_rate_limiter(mut self, rate_limiter: LlmRateLimiter) -> Self {
        self.rate_limiter = rate_limiter;
        self
    }

    fn convert_messages(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system = None;
        let mut converted = Vec::new();

        for msg in messages {
            match msg {
                Message::System { content } => {
                    system = Some(content.clone());
                }
                Message::User { content } => {
                    converted.push(serde_json::json!({
                        "role": "user",
                        "content": content
                    }));
                }
                Message::Assistant { content, tool_calls } => {
                    let mut parts = Vec::new();
                    if let Some(text) = content {
                        parts.push(serde_json::json!({
                            "type": "text",
                            "text": text
                        }));
                    }
                    for tc in tool_calls {
                        parts.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments
                        }));
                    }
                    converted.push(serde_json::json!({
                        "role": "assistant",
                        "content": parts
                    }));
                }
                Message::Tool { tool_call_id, content } => {
                    converted.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content
                        }]
                    }));
                }
            }
        }

        (system, converted)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("anthropic").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let (system, msgs) = Self::convert_messages(messages);

        let tool_defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": options.max_tokens.unwrap_or(4096),
            "messages": msgs,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!(sys);
        }

        if !tool_defs.is_empty() {
            body["tools"] = serde_json::json!(tool_defs);
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let json: serde_json::Value = response.json().await?;

        let mut content_text = String::new();
        let mut tool_calls = Vec::new();

        if let Some(arr) = json["content"].as_array() {
            for block in arr {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            content_text.push_str(text);
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(crate::types::ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            arguments: block.get("input").cloned().unwrap_or_default(),
                        });
                    }
                    _ => {}
                }
            }
        }

        let usage = TokenUsage {
            prompt_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32
                + json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(CompletionResponse {
            content: if content_text.is_empty() { None } else { Some(content_text) },
            tool_calls,
            usage,
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent>> + Send>>> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("anthropic").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let (system, msgs) = Self::convert_messages(messages);

        let tool_defs: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": options.max_tokens.unwrap_or(4096),
            "messages": msgs,
            "stream": true,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::json!(sys);
        }

        if !tool_defs.is_empty() {
            body["tools"] = serde_json::json!(tool_defs);
        }

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let stream = response.bytes_stream();
        let event_stream = futures::StreamExt::map(stream, |chunk| {
            let chunk = chunk.map_err(|e| crate::error::SigmaError::Llm(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk).to_string();

            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        let event_type = json["type"].as_str().unwrap_or("");
                        match event_type {
                            "content_block_start" => {
                                if json["content_block"]["type"] == "tool_use" {
                                    return Ok(LlmEvent::ToolUseStart {
                                        id: json["content_block"]["id"].as_str().unwrap_or("").to_string(),
                                        name: json["content_block"]["name"].as_str().unwrap_or("").to_string(),
                                    });
                                }
                            }
                            "content_block_delta" => {
                                if json["delta"]["type"] == "text_delta" {
                                    if let Some(text) = json["delta"]["text"].as_str() {
                                        return Ok(LlmEvent::ContentDelta(text.to_string()));
                                    }
                                }
                                if json["delta"]["type"] == "input_json_delta" {
                                    if let Some(partial) = json["delta"]["partial_json"].as_str() {
                                        return Ok(LlmEvent::ToolUseDelta {
                                            id: String::new(),
                                            arguments_delta: partial.to_string(),
                                        });
                                    }
                                }
                            }
                            "message_delta" => {
                                return Ok(LlmEvent::Done {
                                    usage: TokenUsage::default(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(LlmEvent::ContentDelta(String::new()))
        });

        Ok(Box::pin(event_stream))
    }
}

// ── Ollama Provider ──

pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
    model: String,
    rate_limiter: LlmRateLimiter,
}

impl OllamaProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            model,
            rate_limiter: LlmRateLimiter::new(),
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn complete(
        &self,
        messages: &[Message],
        _tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("ollama").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": false,
            "options": {
                "temperature": options.temperature.unwrap_or(0.0),
                "num_predict": options.max_tokens.unwrap_or(4096),
            }
        });

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let json: serde_json::Value = response.json().await?;

        let content = json["message"]["content"].as_str().map(|s| s.to_string());

        Ok(CompletionResponse {
            content,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        messages: &[Message],
        _tools: &[crate::types::ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent>> + Send>>> {
        if let RateLimitResult::Limited { retry_after } = self.rate_limiter.check_llm_request("ollama").await {
            return Err(crate::error::SigmaError::RateLimited {
                retry_after_ms: retry_after.as_millis() as u64,
            });
        }

        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect();

        let body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
            "options": {
                "temperature": options.temperature.unwrap_or(0.0),
                "num_predict": options.max_tokens.unwrap_or(4096),
            }
        });

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            let msg = parse_error_message(&text);
            return Err(crate::error::SigmaError::Llm(msg));
        }

        let stream = response.bytes_stream();
        let event_stream = futures::StreamExt::map(stream, |chunk| {
            let chunk = chunk.map_err(|e| crate::error::SigmaError::Llm(e.to_string()))?;
            let text = String::from_utf8_lossy(&chunk).to_string();

            for line in text.lines() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(content) = json["message"]["content"].as_str() {
                        return Ok(LlmEvent::ContentDelta(content.to_string()));
                    }
                    if json["done"].as_bool().unwrap_or(false) {
                        return Ok(LlmEvent::Done {
                            usage: TokenUsage::default(),
                        });
                    }
                }
            }
            Ok(LlmEvent::ContentDelta(String::new()))
        });

        Ok(Box::pin(event_stream))
    }
}

// ── Provider Factory ──

pub fn create_provider(config: &ProviderConfig) -> Box<dyn LlmProvider> {
    match config {
        ProviderConfig::OpenAi { api_key, base_url, model } => {
            Box::new(OpenAiProvider::new(
                api_key.clone(),
                base_url.clone().unwrap_or_else(|| "https://api.openai.com/v1".into()),
                model.clone(),
            ))
        }
        ProviderConfig::Anthropic { api_key, model } => {
            Box::new(AnthropicProvider::new(api_key.clone(), model.clone()))
        }
        ProviderConfig::Ollama { base_url, model } => {
            Box::new(OllamaProvider::new(
                base_url.clone().unwrap_or_else(|| "http://localhost:11434".into()),
                model.clone(),
            ))
        }
        ProviderConfig::Gemini { api_key, model } => {
            Box::new(OpenAiProvider::new(
                api_key.clone(),
                "https://generativelanguage.googleapis.com/v1beta/openai".into(),
                model.clone(),
            ))
        }
    }
}
