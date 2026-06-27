use reqwest::Client;

use grok_search_config::Config;
pub use grok_search_source_core::{
    resolve_content, SourceCaps, SourceExtractor, SourceRouter, NO_SPECIALIST_MATCH,
};
pub use grok_search_types::SourceType;
use grok_search_types::{GrokSearchError, Result};

pub mod arxiv;
pub mod github;
pub mod stackexchange;
pub mod wikipedia;

/// Build the ordered specialist router from runtime config.
pub fn router_from_config(config: &Config) -> SourceRouter {
    SourceRouter::with_extractors(vec![
        Box::new(github::GithubIssueExtractor {
            token: config.github_token.clone(),
        }),
        Box::new(github::GithubPrExtractor {
            token: config.github_token.clone(),
        }),
        Box::new(github::GithubRepoExtractor {
            token: config.github_token.clone(),
        }),
        Box::new(stackexchange::StackExchangeExtractor),
        Box::new(arxiv::ArxivExtractor),
        Box::new(wikipedia::WikipediaExtractor),
    ])
}

/// Issue a JSON GET and normalize transport/status/parse errors into
/// GrokSearchError. headers carries extractor-specific headers such as
/// User-Agent. label distinguishes the source in error messages.
pub async fn get_json(
    client: &Client,
    url: &str,
    headers: &[(reqwest::header::HeaderName, &str)],
    label: &str,
) -> Result<serde_json::Value> {
    let bytes = get_bytes(client, url, headers, label).await?;
    serde_json::from_slice(&bytes)
        .map_err(|err| GrokSearchError::Parse(format!("invalid {label} JSON: {err}")))
}

/// Issue a GET and return the body as UTF-8 (lossy). Same error
/// normalization as get_json.
pub async fn get_text(
    client: &Client,
    url: &str,
    headers: &[(reqwest::header::HeaderName, &str)],
    label: &str,
) -> Result<String> {
    let bytes = get_bytes(client, url, headers, label).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn get_bytes(
    client: &Client,
    url: &str,
    headers: &[(reqwest::header::HeaderName, &str)],
    label: &str,
) -> Result<Vec<u8>> {
    let mut builder = client.get(url);
    for (name, value) in headers {
        builder = builder.header(name.clone(), *value);
    }
    let response = builder.send().await.map_err(|err| {
        if err.is_timeout() {
            GrokSearchError::Timeout(format!("{label} GET timed out: {err}"))
        } else {
            GrokSearchError::Upstream(format!("{label} GET failed: {err}"))
        }
    })?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| GrokSearchError::Upstream(format!("{label} body read failed: {err}")))?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        return Err(GrokSearchError::Upstream(format!(
            "{label} returned HTTP {status}: {text}"
        )));
    }
    Ok(bytes.to_vec())
}
