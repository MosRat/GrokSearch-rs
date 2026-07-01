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

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Clone)]
pub struct OpenAiCompatibleConfig {
    pub api_key: String,
    pub base_url: String,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub max_response_bytes: usize,
}

impl OpenAiCompatibleConfig {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_OPENAI_BASE_URL.to_string(),
            organization: None,
            project: None,
            max_response_bytes: DEFAULT_LLM_MAX_RESPONSE_BYTES,
        }
    }
}

impl fmt::Debug for OpenAiCompatibleConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatibleConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("organization", &self.organization)
            .field("project", &self.project)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct OpenAiCompatibleClient {
    http: Client,
    config: OpenAiCompatibleConfig,
}

impl OpenAiCompatibleClient {
    pub fn new(http: Client, config: OpenAiCompatibleConfig) -> Self {
        Self { http, config }
    }

    pub fn chat_completions_endpoint(&self) -> String {
        endpoint(&self.config.base_url, "/chat/completions")
    }

    pub fn responses_endpoint(&self) -> String {
        endpoint(&self.config.base_url, "/responses")
    }

    pub async fn chat_completion(&self, request: LlmRequest) -> Result<LlmResponse> {
        let payload = openai_chat_completion_request_from_llm(&request);
        let mut builder = self
            .http
            .post(self.chat_completions_endpoint())
            .bearer_auth(&self.config.api_key)
            .json(&payload);
        if let Some(organization) = &self.config.organization {
            builder = builder.header("OpenAI-Organization", organization);
        }
        if let Some(project) = &self.config.project {
            builder = builder.header("OpenAI-Project", project);
        }

        let response = builder.send().await.map_err(|err| {
            grok_search_types::GrokSearchError::Provider(format!(
                "OpenAI-compatible request failed: {err}"
            ))
        })?;
        if !response.status().is_success() {
            return Err(http_error("OpenAI-compatible", response).await);
        }

        let (parsed, raw) = response_json_limited::<OpenAiChatCompletionResponse>(
            "OpenAI-compatible",
            response,
            self.config.max_response_bytes,
        )
        .await?;
        Ok(parsed.into_llm_response_with_raw(Some(raw)))
    }

    pub async fn response(&self, request: LlmRequest) -> Result<OpenAiResponsesResponse> {
        let payload = openai_responses_request_from_llm(&request);
        let response = self
            .http
            .post(self.responses_endpoint())
            .bearer_auth(&self.config.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|err| {
                grok_search_types::GrokSearchError::Provider(format!(
                    "OpenAI Responses request failed: {err}"
                ))
            })?;
        if !response.status().is_success() {
            return Err(http_error("OpenAI Responses", response).await);
        }
        response_json_limited::<OpenAiResponsesResponse>(
            "OpenAI Responses",
            response,
            self.config.max_response_bytes,
        )
        .await
        .map(|(parsed, _raw)| parsed)
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatibleClient {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.chat_completion(request).await
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OpenAiChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<OpenAiToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAiMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OpenAiMessageContent {
    Text(String),
    Blocks(Vec<OpenAiContentBlock>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenAiContentBlock {
    Text { text: String },
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiTool {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub function: OpenAiFunctionTool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiFunctionTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OpenAiToolChoice {
    Named {
        #[serde(rename = "type")]
        choice_type: &'static str,
        function: OpenAiToolChoiceFunction,
    },
    Mode(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiToolChoiceFunction {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiResponsesRequest {
    pub model: String,
    pub input: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAiResponsesTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<OpenAiResponsesToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiResponsesTool {
    #[serde(rename = "type")]
    pub tool_type: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OpenAiResponsesToolChoice {
    Named { name: String },
    Mode(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiChatCompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<OpenAiChatChoice>,
    pub usage: Option<OpenAiUsage>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiChatChoice {
    pub message: OpenAiResponseMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiResponseMessage {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<OpenAiResponseToolCall>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiResponseToolCall {
    pub id: Option<String>,
    pub function: OpenAiResponseFunction,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiResponseFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OpenAiResponsesResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub output: Vec<Value>,
    pub usage: Option<Value>,
}

pub fn openai_chat_completion_request_from_llm(
    request: &LlmRequest,
) -> OpenAiChatCompletionRequest {
    OpenAiChatCompletionRequest {
        model: request.model.clone(),
        messages: request
            .messages
            .iter()
            .map(openai_chat_message_from_llm)
            .collect(),
        tools: request.tools.iter().map(openai_tool_from_llm).collect(),
        tool_choice: request
            .tool_choice
            .as_ref()
            .map(openai_tool_choice_from_llm),
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        top_p: request.top_p,
        stop: request.stop.clone(),
        metadata: request.metadata.clone(),
    }
}

pub fn openai_responses_request_from_llm(request: &LlmRequest) -> OpenAiResponsesRequest {
    OpenAiResponsesRequest {
        model: request.model.clone(),
        input: serde_json::json!(request
            .messages
            .iter()
            .map(openai_responses_message_from_llm)
            .collect::<Vec<_>>()),
        tools: request
            .tools
            .iter()
            .map(openai_responses_tool_from_llm)
            .collect(),
        tool_choice: request
            .tool_choice
            .as_ref()
            .map(openai_responses_tool_choice_from_llm),
        max_output_tokens: request.max_tokens,
        temperature: request.temperature,
        top_p: request.top_p,
        metadata: request.metadata.clone(),
    }
}

fn openai_chat_message_from_llm(message: &LlmMessage) -> OpenAiChatMessage {
    OpenAiChatMessage {
        role: match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        }
        .to_string(),
        content: openai_content_from_llm(&message.content),
        name: message.name.clone(),
        tool_call_id: message.tool_call_id.clone().or_else(|| {
            message.content.iter().find_map(|block| match block {
                LlmContentBlock::ToolResult { tool_call_id, .. } => tool_call_id.clone(),
                _ => None,
            })
        }),
    }
}

fn openai_responses_message_from_llm(message: &LlmMessage) -> Value {
    serde_json::json!({
        "role": match message.role {
            LlmRole::System => "system",
            LlmRole::User => "user",
            LlmRole::Assistant => "assistant",
            LlmRole::Tool => "tool",
        },
        "content": message.content.iter().map(openai_responses_content_from_llm).collect::<Vec<_>>(),
    })
}

fn openai_content_from_llm(content: &[LlmContentBlock]) -> Option<OpenAiMessageContent> {
    if content.is_empty() {
        return None;
    }

    if content.len() == 1 {
        if let Some(text) = content[0].as_text() {
            return Some(OpenAiMessageContent::Text(text.to_string()));
        }
    }

    let blocks = content
        .iter()
        .filter_map(|block| match block {
            LlmContentBlock::Text { text } => Some(OpenAiContentBlock::Text { text: text.clone() }),
            LlmContentBlock::ImageUrl { url, detail } => Some(OpenAiContentBlock::ImageUrl {
                image_url: OpenAiImageUrl {
                    url: url.clone(),
                    detail: detail.clone(),
                },
            }),
            LlmContentBlock::ImageBase64 {
                media_type,
                data,
                detail,
            } => Some(OpenAiContentBlock::ImageUrl {
                image_url: OpenAiImageUrl {
                    url: data_url(media_type, data),
                    detail: detail.clone(),
                },
            }),
            LlmContentBlock::ToolResult { content, .. } => Some(OpenAiContentBlock::Text {
                text: content.clone(),
            }),
        })
        .collect();
    Some(OpenAiMessageContent::Blocks(blocks))
}

fn openai_responses_content_from_llm(block: &LlmContentBlock) -> Value {
    match block {
        LlmContentBlock::Text { text } => serde_json::json!({"type": "input_text", "text": text}),
        LlmContentBlock::ImageUrl { url, detail } => {
            let mut value = serde_json::json!({"type": "input_image", "image_url": url});
            if let Some(detail) = detail {
                value["detail"] = Value::String(detail.clone());
            }
            value
        }
        LlmContentBlock::ImageBase64 {
            media_type,
            data,
            detail,
        } => {
            let mut value =
                serde_json::json!({"type": "input_image", "image_url": data_url(media_type, data)});
            if let Some(detail) = detail {
                value["detail"] = Value::String(detail.clone());
            }
            value
        }
        LlmContentBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => serde_json::json!({
            "type": "function_call_output",
            "call_id": tool_call_id,
            "output": content,
            "is_error": is_error,
        }),
    }
}

fn data_url(media_type: &str, data: &str) -> String {
    format!("data:{media_type};base64,{data}")
}

fn openai_tool_from_llm(tool: &LlmTool) -> OpenAiTool {
    OpenAiTool {
        tool_type: "function",
        function: OpenAiFunctionTool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        },
    }
}

fn openai_responses_tool_from_llm(tool: &LlmTool) -> OpenAiResponsesTool {
    OpenAiResponsesTool {
        tool_type: "function",
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.input_schema.clone(),
    }
}

fn openai_tool_choice_from_llm(choice: &LlmToolChoice) -> OpenAiToolChoice {
    match choice {
        LlmToolChoice::Auto => OpenAiToolChoice::Mode("auto".to_string()),
        LlmToolChoice::None => OpenAiToolChoice::Mode("none".to_string()),
        LlmToolChoice::Required => OpenAiToolChoice::Mode("required".to_string()),
        LlmToolChoice::Tool { name } => OpenAiToolChoice::Named {
            choice_type: "function",
            function: OpenAiToolChoiceFunction { name: name.clone() },
        },
    }
}

fn openai_responses_tool_choice_from_llm(choice: &LlmToolChoice) -> OpenAiResponsesToolChoice {
    match choice {
        LlmToolChoice::Auto => OpenAiResponsesToolChoice::Mode("auto".to_string()),
        LlmToolChoice::None => OpenAiResponsesToolChoice::Mode("none".to_string()),
        LlmToolChoice::Required => OpenAiResponsesToolChoice::Mode("required".to_string()),
        LlmToolChoice::Tool { name } => OpenAiResponsesToolChoice::Named { name: name.clone() },
    }
}

impl OpenAiChatCompletionResponse {
    pub fn into_llm_response(self) -> LlmResponse {
        self.into_llm_response_with_raw(None)
    }

    pub fn into_llm_response_with_raw(self, raw: Option<Value>) -> LlmResponse {
        let first_choice = self.choices.into_iter().next();
        let (content, tool_calls, stop_reason) = first_choice.map_or_else(
            || (Vec::new(), Vec::new(), None),
            |choice| {
                let content = choice
                    .message
                    .content
                    .filter(|text| !text.is_empty())
                    .map(LlmContentBlock::text)
                    .into_iter()
                    .collect();
                let tool_calls = choice
                    .message
                    .tool_calls
                    .into_iter()
                    .map(|tool_call| LlmToolCall {
                        id: tool_call.id,
                        name: tool_call.function.name,
                        arguments: serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or(Value::String(tool_call.function.arguments)),
                    })
                    .collect();
                (content, tool_calls, choice.finish_reason)
            },
        );

        LlmResponse {
            provider: LlmProviderKind::OpenAiCompatible,
            id: self.id,
            model: self.model,
            content,
            tool_calls,
            stop_reason,
            usage: self.usage.map(|usage| LlmUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
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
    fn chat_request_maps_tools_and_tool_choice() {
        let mut request = LlmRequest::new(
            "gpt-test",
            vec![LlmMessage::text(LlmRole::User, "find papers")],
        );
        request.tools.push(LlmTool {
            name: "search".to_string(),
            description: Some("Search the index".to_string()),
            input_schema: json!({"type": "object"}),
        });
        request.tool_choice = Some(LlmToolChoice::Tool {
            name: "search".to_string(),
        });

        let payload = openai_chat_completion_request_from_llm(&request);
        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["model"], "gpt-test");
        assert_eq!(value["tools"][0]["type"], "function");
        assert_eq!(value["tools"][0]["function"]["name"], "search");
        assert_eq!(value["tool_choice"]["type"], "function");
        assert_eq!(value["tool_choice"]["function"]["name"], "search");
    }

    #[test]
    fn responses_request_uses_output_token_name() {
        let mut request = LlmRequest::new(
            "gpt-test",
            vec![LlmMessage::text(LlmRole::User, "summarize")],
        );
        request.max_tokens = Some(128);
        let value = serde_json::to_value(openai_responses_request_from_llm(&request)).unwrap();
        assert_eq!(value["max_output_tokens"], 128);
    }

    #[test]
    fn chat_and_responses_request_map_base64_images_as_data_urls() {
        let request = LlmRequest::new(
            "gpt-test",
            vec![LlmMessage::new(
                LlmRole::User,
                vec![LlmContentBlock::image_base64(
                    "image/png",
                    "aGVsbG8=",
                    Some("low".to_string()),
                )],
            )],
        );

        let chat = serde_json::to_value(openai_chat_completion_request_from_llm(&request)).unwrap();
        assert_eq!(
            chat["messages"][0]["content"][0]["image_url"]["url"],
            "data:image/png;base64,aGVsbG8="
        );
        let responses = serde_json::to_value(openai_responses_request_from_llm(&request)).unwrap();
        assert_eq!(
            responses["input"][0]["content"][0]["image_url"],
            "data:image/png;base64,aGVsbG8="
        );
    }

    #[test]
    fn chat_response_maps_content_usage_and_tool_calls() {
        let response: OpenAiChatCompletionResponse = serde_json::from_value(json!({
            "id": "chatcmpl_1",
            "model": "gpt-test",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "hello",
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {
                            "name": "search",
                            "arguments": "{\"query\":\"rust\"}"
                        }
                    }]
                }
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "total_tokens": 14
            }
        }))
        .unwrap();

        let normalized = response.into_llm_response();
        assert_eq!(normalized.provider, LlmProviderKind::OpenAiCompatible);
        assert_eq!(normalized.content[0].as_text(), Some("hello"));
        assert_eq!(normalized.tool_calls[0].name, "search");
        assert_eq!(normalized.tool_calls[0].arguments["query"], "rust");
        assert_eq!(normalized.usage.unwrap().total_tokens, Some(14));
    }
}
