use std::sync::Arc;

use grok_search_provider_core::SourceProvider;
use grok_search_source_core::{resolve_content, SourceCaps, SourceRouter};
use grok_search_types::model::source::Source;

use crate::fetch::generic_source_fetch;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn enrich_sources(
    sources: Vec<Source>,
    deadline: tokio::time::Instant,
    client: &reqwest::Client,
    router: &Arc<SourceRouter>,
    caps: SourceCaps,
    concurrency: usize,
    max_chars: usize,
    max_sources: usize,
    primary: Option<Arc<dyn SourceProvider>>,
    fallback: Option<Arc<dyn SourceProvider>>,
) -> Vec<Source> {
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut set: tokio::task::JoinSet<(usize, Option<String>)> = tokio::task::JoinSet::new();

    for (idx, source) in sources.iter().enumerate().take(max_sources) {
        let permit = Arc::clone(&sem);
        let url_str = source.url.clone();
        let client = client.clone();
        let router = Arc::clone(router);
        let caps = caps.clone();
        let primary = primary.clone();
        let fallback = fallback.clone();

        set.spawn(async move {
            let _permit = permit.acquire_owned().await.ok();
            let content = match url::Url::parse(&url_str) {
                Err(_) => Some(format!(
                    "_Failed to retrieve: invalid_url_\n\nSource: {url_str}"
                )),
                Ok(parsed) => {
                    let future = resolve_content(&client, &parsed, &router, &caps);
                    match tokio::time::timeout_at(deadline, future).await {
                        Ok(Ok((md, _kind))) => {
                            let truncated: String = md.chars().take(max_chars).collect();
                            Some(truncated)
                        }
                        Ok(Err(reason)) => {
                            let generic = generic_source_fetch(&primary, &fallback, &url_str);
                            match tokio::time::timeout_at(deadline, generic).await {
                                Ok(Ok(md)) => {
                                    let truncated: String = md.chars().take(max_chars).collect();
                                    Some(truncated)
                                }
                                Ok(Err(_)) => Some(format!(
                                    "_Failed to retrieve: {reason}_\n\nSource: {url_str}"
                                )),
                                Err(_elapsed) => Some(format!(
                                    "_Failed to retrieve: timeout_\n\nSource: {url_str}"
                                )),
                            }
                        }
                        Err(_elapsed) => Some(format!(
                            "_Failed to retrieve: timeout_\n\nSource: {url_str}"
                        )),
                    }
                }
            };
            (idx, content)
        });
    }

    let mut results: Vec<(usize, Option<String>)> = Vec::with_capacity(sources.len());
    while let Some(res) = set.join_next().await {
        if let Ok(pair) = res {
            results.push(pair);
        }
    }

    results.sort_by_key(|(idx, _)| *idx);
    let mut out = sources;
    for (idx, content) in results {
        out[idx].content = content;
    }
    out
}
