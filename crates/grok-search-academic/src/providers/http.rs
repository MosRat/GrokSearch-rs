use std::time::Duration;

use grok_search_types::{GrokSearchError, Result};
use reqwest::header::{RETRY_AFTER, USER_AGENT};
use serde_json::Value;

use crate::service::UA;

pub(super) fn retry_after_delay(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

pub(super) struct GetBytesFailure {
    pub(super) status: Option<u16>,
    pub(super) retry_after: Option<Duration>,
    pub(super) error: GrokSearchError,
}

pub(super) async fn get_bytes_with_status(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    max_response_bytes: usize,
) -> std::result::Result<Vec<u8>, GetBytesFailure> {
    let response = client
        .get(url)
        .header(USER_AGENT, UA)
        .send()
        .await
        .map_err(|err| GetBytesFailure {
            status: None,
            retry_after: None,
            error: if err.is_timeout() {
                GrokSearchError::Timeout(format!("{label} GET timed out: {err}"))
            } else {
                GrokSearchError::Upstream(format!("{label} GET failed: {err}"))
            },
        })?;
    let status = response.status();
    let retry_after = retry_after_delay(&response);
    let bytes = read_response_bytes_limited(response, label, max_response_bytes)
        .await
        .map_err(|error| GetBytesFailure {
            status: None,
            retry_after: None,
            error,
        })?;
    if !status.is_success() {
        return Err(GetBytesFailure {
            status: Some(status.as_u16()),
            retry_after,
            error: GrokSearchError::Upstream(format!(
                "{label} returned HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            )),
        });
    }
    Ok(bytes)
}

pub(super) fn retry_delay(retry_after: Option<Duration>, fallback: Duration) -> Duration {
    retry_after.unwrap_or(fallback)
}

pub(super) struct GetJsonFailure {
    pub(super) status: Option<u16>,
    pub(super) retry_after: Option<Duration>,
    pub(super) error: GrokSearchError,
}

pub(super) async fn get_json_with_status(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    max_response_bytes: usize,
) -> std::result::Result<Value, GetJsonFailure> {
    let response = client
        .get(url)
        .header(USER_AGENT, UA)
        .send()
        .await
        .map_err(|err| GetJsonFailure {
            status: None,
            retry_after: None,
            error: if err.is_timeout() {
                GrokSearchError::Timeout(format!("{label} GET timed out: {err}"))
            } else {
                GrokSearchError::Upstream(format!("{label} GET failed: {err}"))
            },
        })?;
    let status = response.status();
    let retry_after = retry_after_delay(&response);
    let bytes = read_response_bytes_limited(response, label, max_response_bytes)
        .await
        .map_err(|error| GetJsonFailure {
            status: None,
            retry_after: None,
            error,
        })?;
    if !status.is_success() {
        return Err(GetJsonFailure {
            status: Some(status.as_u16()),
            retry_after,
            error: GrokSearchError::Upstream(format!(
                "{label} returned HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            )),
        });
    }
    serde_json::from_slice(&bytes).map_err(|err| GetJsonFailure {
        status: None,
        retry_after: None,
        error: GrokSearchError::Parse(format!("invalid {label} JSON: {err}")),
    })
}

pub(super) async fn read_response_bytes_limited(
    mut response: reqwest::Response,
    label: &str,
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| GrokSearchError::Upstream(format!("{label} body read failed: {err}")))?
    {
        if out.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(GrokSearchError::Upstream(format!(
                "{label} response exceeded max_response_bytes={max_response_bytes}"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
