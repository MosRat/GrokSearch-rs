use crate::protocol::{LlmRequest, LlmResponse};
use async_trait::async_trait;
use grok_search_types::{GrokSearchError, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_LLM_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    OpenAiCompatible,
    AnthropicCompatible,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
}

pub(crate) fn endpoint(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) async fn http_error(provider: &str, response: reqwest::Response) -> GrokSearchError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    GrokSearchError::Provider(format!(
        "{provider} HTTP {status}: {}",
        truncate_for_error(&body, 1024)
    ))
}

pub(crate) fn parse_error(provider: &str, err: impl std::fmt::Display) -> GrokSearchError {
    GrokSearchError::Parse(format!("{provider} response JSON parse failed: {err}"))
}

pub(crate) async fn response_json_limited<T>(
    provider: &str,
    response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<(T, Value)>
where
    T: DeserializeOwned,
{
    let bytes = response_bytes_limited(provider, response, max_response_bytes).await?;
    let raw: Value = serde_json::from_slice(&bytes).map_err(|err| parse_error(provider, err))?;
    let parsed = serde_json::from_value(raw.clone()).map_err(|err| parse_error(provider, err))?;
    Ok((parsed, raw))
}

async fn response_bytes_limited(
    provider: &str,
    mut response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length as usize > max_response_bytes)
    {
        return Err(GrokSearchError::Upstream(format!(
            "{provider} response exceeded max_response_bytes={max_response_bytes}"
        )));
    }

    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| GrokSearchError::Upstream(format!("{provider} body read failed: {err}")))?
    {
        if out.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(GrokSearchError::Upstream(format!(
                "{provider} response exceeded max_response_bytes={max_response_bytes}"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn truncate_for_error(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_trims_slashes() {
        assert_eq!(
            endpoint("https://example.com/v1/", "/chat/completions"),
            "https://example.com/v1/chat/completions"
        );
    }
}
