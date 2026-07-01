use crate::client::{
    endpoint, http_error, response_json_limited, LlmClient, LlmProviderKind,
    DEFAULT_LLM_MAX_RESPONSE_BYTES,
};
use crate::protocol::{
    LlmContentBlock, LlmMessage, LlmRequest, LlmResponse, LlmRole, LlmTool, LlmToolCall,
    LlmToolChoice, LlmUsage,
};
use async_trait::async_trait;
use grok_search_types::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_MINIMAX_ANTHROPIC_BASE_URL: &str = "https://api.minimaxi.com/anthropic";

#[derive(Clone)]
pub struct AnthropicClientConfig {
    pub api_key: String,
    pub base_url: String,
    pub anthropic_version: String,
    pub beta: Vec<String>,
    pub auth_scheme: AnthropicAuthScheme,
    pub max_response_bytes: usize,
}

impl AnthropicClientConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_ANTHROPIC_BASE_URL.to_string(),
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            beta: Vec::new(),
            auth_scheme: AnthropicAuthScheme::XApiKey,
            max_response_bytes: DEFAULT_LLM_MAX_RESPONSE_BYTES,
        }
    }

    pub fn minimax(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_MINIMAX_ANTHROPIC_BASE_URL.to_string(),
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.to_string(),
            beta: Vec::new(),
            auth_scheme: AnthropicAuthScheme::Bearer,
            max_response_bytes: DEFAULT_LLM_MAX_RESPONSE_BYTES,
        }
    }
}

impl fmt::Debug for AnthropicClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicClientConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("anthropic_version", &self.anthropic_version)
            .field("beta", &self.beta)
            .field("auth_scheme", &self.auth_scheme)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicAuthScheme {
    XApiKey,
    Bearer,
    Both,
}

#[derive(Clone, Debug)]
pub struct AnthropicMessagesClient {
    http: Client,
    config: AnthropicClientConfig,
}

impl AnthropicMessagesClient {
    pub fn new(http: Client, config: AnthropicClientConfig) -> Self {
        Self { http, config }
    }

    pub fn messages_endpoint(&self) -> String {
        endpoint(&self.config.base_url, "/v1/messages")
    }

    pub async fn message(&self, request: LlmRequest) -> Result<LlmResponse> {
        let payload = anthropic_messages_request_from_llm(&request);
        let mut builder = self
            .http
            .post(self.messages_endpoint())
            .header("anthropic-version", &self.config.anthropic_version)
            .json(&payload);
        builder = match self.config.auth_scheme {
            AnthropicAuthScheme::XApiKey => builder.header("x-api-key", &self.config.api_key),
            AnthropicAuthScheme::Bearer => builder.bearer_auth(&self.config.api_key),
            AnthropicAuthScheme::Both => builder
                .header("x-api-key", &self.config.api_key)
                .bearer_auth(&self.config.api_key),
        };
        if !self.config.beta.is_empty() {
            builder = builder.header("anthropic-beta", self.config.beta.join(","));
        }

        let response = builder.send().await.map_err(|err| {
            grok_search_types::GrokSearchError::Provider(format!(
                "Anthropic-compatible request failed: {err}"
            ))
        })?;
        if !response.status().is_success() {
            return Err(http_error("Anthropic-compatible", response).await);
        }
        let (parsed, raw) = response_json_limited::<AnthropicMessagesResponse>(
            "Anthropic-compatible",
            response,
            self.config.max_response_bytes,
        )
        .await?;
        Ok(parsed.into_llm_response_with_raw(Some(raw)))
    }
}

#[async_trait]
impl LlmClient for AnthropicMessagesClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.message(request).await
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnthropicMessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicContentBlock {
    Text {
        text: String,
    },
    Image {
        source: AnthropicImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: Option<String>,
    },
    RedactedThinking {
        #[serde(default)]
        data: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnthropicTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnthropicToolChoice {
    Auto,
    None,
    Any,
    Tool { name: String },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AnthropicMessagesResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub content: Vec<AnthropicContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct AnthropicUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub fn anthropic_messages_request_from_llm(request: &LlmRequest) -> AnthropicMessagesRequest {
    let mut system_parts = request.system.iter().cloned().collect::<Vec<_>>();
    let mut messages = Vec::new();

    for message in &request.messages {
        match message.role {
            LlmRole::System => {
                let text = text_from_blocks(&message.content);
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            LlmRole::User | LlmRole::Assistant | LlmRole::Tool => {
                messages.push(anthropic_message_from_llm(message));
            }
        }
    }

    AnthropicMessagesRequest {
        model: request.model.clone(),
        max_tokens: request.max_tokens.unwrap_or(1024),
        messages,
        system: if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n\n"))
        },
        tools: request.tools.iter().map(anthropic_tool_from_llm).collect(),
        tool_choice: request
            .tool_choice
            .as_ref()
            .map(anthropic_tool_choice_from_llm),
        temperature: request.temperature,
        top_p: request.top_p,
        stop_sequences: request.stop.clone(),
        metadata: request.metadata.clone(),
    }
}

fn anthropic_message_from_llm(message: &LlmMessage) -> AnthropicMessage {
    AnthropicMessage {
        role: match message.role {
            LlmRole::Assistant => "assistant",
            LlmRole::System | LlmRole::User | LlmRole::Tool => "user",
        }
        .to_string(),
        content: message
            .content
            .iter()
            .filter_map(|block| match block {
                LlmContentBlock::Text { text } => {
                    Some(AnthropicContentBlock::Text { text: text.clone() })
                }
                LlmContentBlock::ImageUrl { .. } => None,
                LlmContentBlock::ImageBase64 {
                    media_type, data, ..
                } => Some(AnthropicContentBlock::Image {
                    source: AnthropicImageSource {
                        source_type: "base64".to_string(),
                        media_type: media_type.clone(),
                        data: data.clone(),
                    },
                }),
                LlmContentBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => Some(AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id
                        .clone()
                        .or_else(|| message.tool_call_id.clone())
                        .unwrap_or_default(),
                    content: content.clone(),
                    is_error: *is_error,
                }),
            })
            .collect(),
    }
}

fn anthropic_tool_from_llm(tool: &LlmTool) -> AnthropicTool {
    AnthropicTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
    }
}

fn anthropic_tool_choice_from_llm(choice: &LlmToolChoice) -> AnthropicToolChoice {
    match choice {
        LlmToolChoice::Auto => AnthropicToolChoice::Auto,
        LlmToolChoice::None => AnthropicToolChoice::None,
        LlmToolChoice::Required => AnthropicToolChoice::Any,
        LlmToolChoice::Tool { name } => AnthropicToolChoice::Tool { name: name.clone() },
    }
}

fn text_from_blocks(content: &[LlmContentBlock]) -> String {
    content
        .iter()
        .filter_map(LlmContentBlock::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

impl AnthropicMessagesResponse {
    pub fn into_llm_response(self) -> LlmResponse {
        self.into_llm_response_with_raw(None)
    }

    pub fn into_llm_response_with_raw(self, raw: Option<Value>) -> LlmResponse {
        let mut content = Vec::new();
        let mut tool_calls = Vec::new();

        for block in self.content {
            match block {
                AnthropicContentBlock::Text { text } => content.push(LlmContentBlock::text(text)),
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    tool_calls.push(LlmToolCall {
                        id: Some(id),
                        name,
                        arguments: input,
                    });
                }
                AnthropicContentBlock::ToolResult { .. } => {}
                AnthropicContentBlock::Image { .. } => {}
                AnthropicContentBlock::Thinking { .. }
                | AnthropicContentBlock::RedactedThinking { .. }
                | AnthropicContentBlock::Unknown => {}
            }
        }

        LlmResponse {
            provider: LlmProviderKind::AnthropicCompatible,
            id: self.id,
            model: self.model,
            content,
            tool_calls,
            stop_reason: self.stop_reason,
            usage: self.usage.map(|usage| {
                let total_tokens = match (usage.input_tokens, usage.output_tokens) {
                    (Some(input), Some(output)) => Some(input + output),
                    _ => None,
                };
                LlmUsage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens,
                }
            }),
            raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_request_moves_system_out_of_messages() {
        let request = LlmRequest::new(
            "claude-test",
            vec![
                LlmMessage::text(LlmRole::System, "be terse"),
                LlmMessage::text(LlmRole::User, "hello"),
            ],
        );

        let payload = anthropic_messages_request_from_llm(&request);
        assert_eq!(payload.system.as_deref(), Some("be terse"));
        assert_eq!(payload.messages.len(), 1);
        assert_eq!(payload.messages[0].role, "user");
    }

    #[test]
    fn messages_request_maps_tools_and_required_choice() {
        let mut request = LlmRequest::new(
            "claude-test",
            vec![LlmMessage::text(LlmRole::User, "search")],
        );
        request.tools.push(LlmTool {
            name: "search".to_string(),
            description: None,
            input_schema: json!({"type": "object"}),
        });
        request.tool_choice = Some(LlmToolChoice::Required);

        let value = serde_json::to_value(anthropic_messages_request_from_llm(&request)).unwrap();
        assert_eq!(value["tools"][0]["name"], "search");
        assert_eq!(value["tool_choice"]["type"], "any");
    }

    #[test]
    fn messages_request_maps_base64_image_blocks() {
        let request = LlmRequest::new(
            "MiniMax-M3",
            vec![LlmMessage::new(
                LlmRole::User,
                vec![
                    LlmContentBlock::text("inspect"),
                    LlmContentBlock::image_base64("image/png", "aGVsbG8=", Some("low".to_string())),
                ],
            )],
        );

        let value = serde_json::to_value(anthropic_messages_request_from_llm(&request)).unwrap();
        let image = &value["messages"][0]["content"][1];
        assert_eq!(image["type"], "image");
        assert_eq!(image["source"]["type"], "base64");
        assert_eq!(image["source"]["media_type"], "image/png");
        assert_eq!(image["source"]["data"], "aGVsbG8=");
    }

    #[test]
    fn messages_response_maps_text_tool_use_and_usage() {
        let response: AnthropicMessagesResponse = serde_json::from_value(json!({
            "id": "msg_1",
            "model": "claude-test",
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "tool_use", "id": "toolu_1", "name": "search", "input": {"query": "rust"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 11, "output_tokens": 3}
        }))
        .unwrap();

        let normalized = response.into_llm_response();
        assert_eq!(normalized.provider, LlmProviderKind::AnthropicCompatible);
        assert_eq!(normalized.content[0].as_text(), Some("hello"));
        assert_eq!(normalized.tool_calls[0].id.as_deref(), Some("toolu_1"));
        assert_eq!(normalized.tool_calls[0].arguments["query"], "rust");
        assert_eq!(normalized.usage.unwrap().total_tokens, Some(14));
    }

    #[test]
    fn minimax_config_uses_bearer_auth_and_anthropic_base_url() {
        let config = AnthropicClientConfig::minimax("secret");
        assert_eq!(config.auth_scheme, AnthropicAuthScheme::Bearer);
        assert_eq!(config.base_url, DEFAULT_MINIMAX_ANTHROPIC_BASE_URL);
        let client = AnthropicMessagesClient::new(reqwest::Client::new(), config);
        assert_eq!(
            client.messages_endpoint(),
            "https://api.minimaxi.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn messages_response_ignores_thinking_blocks() {
        let response: AnthropicMessagesResponse = serde_json::from_value(json!({
            "id": "msg_1",
            "model": "MiniMax-M3",
            "content": [
                {"type": "thinking", "thinking": "private chain", "signature": "sig"},
                {"type": "text", "text": "{\"ok\":true}"}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 20, "output_tokens": 5}
        }))
        .unwrap();

        let normalized = response.into_llm_response();
        assert_eq!(normalized.content.len(), 1);
        assert_eq!(normalized.content[0].as_text(), Some("{\"ok\":true}"));
    }
}
