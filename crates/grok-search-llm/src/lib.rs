//! Provider-neutral LLM protocol types and compatible client adapters.
//!
//! This crate intentionally keeps the public boundary small: callers build a
//! [`LlmRequest`], send it through an [`LlmClient`], and receive normalized text,
//! tool call, stop reason, and usage metadata. Provider-specific modules expose
//! the wire protocol structs used by OpenAI-compatible Chat/Responses APIs and
//! Anthropic-compatible Messages APIs so future SDK adapters can share the same
//! shape without pushing SDK types through the rest of the workspace.

pub mod anthropic;
pub mod client;
pub mod openai;
pub mod protocol;

pub use anthropic::{
    anthropic_messages_request_from_llm, AnthropicAuthScheme, AnthropicClientConfig,
    AnthropicMessage, AnthropicMessagesClient, AnthropicMessagesRequest, AnthropicMessagesResponse,
    DEFAULT_MINIMAX_ANTHROPIC_BASE_URL,
};
pub use client::{LlmClient, LlmProviderKind};
pub use openai::{
    openai_chat_completion_request_from_llm, openai_responses_request_from_llm,
    OpenAiChatCompletionRequest, OpenAiChatCompletionResponse, OpenAiCompatibleClient,
    OpenAiCompatibleConfig, OpenAiResponsesRequest, OpenAiResponsesResponse,
};
pub use protocol::{
    LlmContentBlock, LlmMessage, LlmRequest, LlmResponse, LlmRole, LlmTool, LlmToolCall,
    LlmToolChoice, LlmUsage,
};
