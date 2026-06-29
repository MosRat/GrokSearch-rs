use std::sync::Arc;
use std::time::Instant;

use serde_json::json;

use crate::service::{SearchService, SourceProvider};
use grok_search_net::url_policy::validate_public_http_url;
use grok_search_source_core::{resolve_content, SourceCaps, SourceType, NO_SPECIALIST_MATCH};
use grok_search_types::model::source::Source;
use grok_search_types::model::tool::WebFetchOutput;
use grok_search_types::{GrokSearchError, Result};

impl SearchService {
    pub async fn web_fetch(&self, url: &str, max_chars: Option<usize>) -> Result<WebFetchOutput> {
        let op_start = Instant::now();
        let request_id = self.logger.request_id();
        self.logger.event(
            &request_id,
            "debug",
            "web_fetch.start",
            Some("web_fetch"),
            None,
            json!({
                "url": summarize_url(url),
                "max_chars": max_chars,
            }),
        );
        let result = self.web_fetch_inner(url, max_chars).await;
        match &result {
            Ok(output) => self.logger.event(
                &request_id,
                "debug",
                "web_fetch.success",
                Some("web_fetch"),
                Some(op_start.elapsed()),
                json!({
                    "url": summarize_url(&output.url),
                    "source_type": format!("{:?}", output.source_type),
                    "original_length": output.original_length,
                    "truncated": output.truncated,
                    "fallback_reason": output.fallback_reason,
                }),
            ),
            Err(err) => self.logger.error(
                &request_id,
                "web_fetch.error",
                Some("web_fetch"),
                Some(op_start.elapsed()),
                err,
                json!({ "url": summarize_url(url) }),
            ),
        }
        result
    }

    async fn web_fetch_inner(&self, url: &str, max_chars: Option<usize>) -> Result<WebFetchOutput> {
        validate_public_http_url(url)?;
        let effective_limit = max_chars.or(self.config.fetch_max_chars);

        let (content, source_type, fallback_reason) = match url::Url::parse(url) {
            Ok(parsed) => {
                match resolve_content(
                    &self.http_client,
                    &parsed,
                    self.source_router.as_ref(),
                    &SourceCaps {
                        max_answers: self.config.source_max_answers,
                        max_comments: self.config.source_max_comments,
                    },
                )
                .await
                {
                    Ok((content, kind)) => (content, kind, None),
                    Err(reason) if reason == NO_SPECIALIST_MATCH => {
                        let generic = self.web_fetch_raw(url).await?;
                        (generic, SourceType::Generic, None)
                    }
                    Err(reason) => {
                        let generic = self.web_fetch_raw(url).await?;
                        (generic, SourceType::Generic, Some(reason))
                    }
                }
            }
            Err(_) => {
                let generic = self.web_fetch_raw(url).await?;
                (generic, SourceType::Generic, None)
            }
        };

        Ok(apply_fetch_limit(
            url,
            content,
            effective_limit,
            source_type,
            fallback_reason,
        ))
    }

    async fn web_fetch_raw(&self, url: &str) -> Result<String> {
        generic_source_fetch(&self.sources, &self.fallback_sources, url).await
    }

    pub async fn web_map(&self, url: &str, max_results: usize) -> Result<Vec<Source>> {
        let op_start = Instant::now();
        let request_id = self.logger.request_id();
        self.logger.event(
            &request_id,
            "debug",
            "web_map.start",
            Some("web_map"),
            None,
            json!({
                "url": summarize_url(url),
                "max_results": max_results,
            }),
        );
        let result = async {
            validate_public_http_url(url)?;
            self.sources
                .as_ref()
                .ok_or(GrokSearchError::MissingConfig("TAVILY_API_KEY"))?
                .map(url, max_results)
                .await
        }
        .await;
        match &result {
            Ok(sources) => self.logger.event(
                &request_id,
                "debug",
                "web_map.success",
                Some("web_map"),
                Some(op_start.elapsed()),
                json!({
                    "url": summarize_url(url),
                    "sources_count": sources.len(),
                }),
            ),
            Err(err) => self.logger.error(
                &request_id,
                "web_map.error",
                Some("web_map"),
                Some(op_start.elapsed()),
                err,
                json!({ "url": summarize_url(url), "max_results": max_results }),
            ),
        }
        result
    }
}

fn apply_fetch_limit(
    url: &str,
    mut content: String,
    max_chars: Option<usize>,
    source_type: SourceType,
    fallback_reason: Option<String>,
) -> WebFetchOutput {
    let Some(limit) = max_chars else {
        let original_length = content.chars().count();
        return WebFetchOutput {
            url: url.to_string(),
            content,
            original_length,
            truncated: false,
            source_type,
            fallback_reason,
        };
    };

    let mut count = 0usize;
    let mut cutoff: Option<usize> = None;
    for (byte_idx, _) in content.char_indices() {
        if count == limit {
            cutoff = Some(byte_idx);
            break;
        }
        count += 1;
    }

    match cutoff {
        Some(byte_idx) => {
            let extra = content[byte_idx..].chars().count();
            content.truncate(byte_idx);
            WebFetchOutput {
                url: url.to_string(),
                content,
                original_length: limit + extra,
                truncated: true,
                source_type,
                fallback_reason,
            }
        }
        None => WebFetchOutput {
            url: url.to_string(),
            content,
            original_length: count,
            truncated: false,
            source_type,
            fallback_reason,
        },
    }
}

pub(crate) fn summarize_url(raw: &str) -> serde_json::Value {
    match url::Url::parse(raw) {
        Ok(parsed) => serde_json::json!({
            "scheme": parsed.scheme(),
            "host": parsed.host_str(),
            "path": parsed.path(),
        }),
        Err(_) => serde_json::json!({
            "invalid": true,
            "length": raw.len(),
        }),
    }
}

pub(crate) async fn generic_source_fetch(
    primary: &Option<Arc<dyn SourceProvider>>,
    fallback: &Option<Arc<dyn SourceProvider>>,
    url: &str,
) -> Result<String> {
    if let Some(provider) = primary {
        if let Ok(content) = provider.fetch(url).await {
            if !content.trim().is_empty() {
                return Ok(content);
            }
        }
    }
    if let Some(provider) = fallback {
        return provider.fetch(url).await;
    }
    Err(GrokSearchError::MissingConfig(
        "TAVILY_API_KEY or FIRECRAWL_API_KEY",
    ))
}
