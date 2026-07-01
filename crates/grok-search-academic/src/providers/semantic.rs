use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use grok_search_net::http::{get_json_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicPaper, AcademicSearchInput, GrokSearchError, Result,
};
use reqwest::header::USER_AGENT;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::{sleep_until, Instant};
use url::Url;

use super::http::{read_response_bytes_limited, retry_after_delay};
use super::rate_limit::wait_for_global_provider_rate_limit;
use super::sort_is;
use crate::service::{as_u32, source, UA};

#[derive(Clone)]
pub(crate) struct SemanticProvider {
    client: reqwest::Client,
    api_key: Option<String>,
    limiter: Arc<Mutex<Option<Instant>>>,
    max_response_bytes: usize,
}

impl SemanticProvider {
    #[allow(dead_code)]
    pub(crate) fn new(client: reqwest::Client, api_key: Option<String>) -> Self {
        Self::new_with_limit(client, api_key, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub(crate) fn new_with_limit(
        client: reqwest::Client,
        api_key: Option<String>,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            client,
            api_key,
            limiter: Arc::new(Mutex::new(None)),
            max_response_bytes,
        }
    }

    pub(super) async fn get_json_with_optional_key(&self, url: &str, label: &str) -> Result<Value> {
        let mut last_rate_limit = None;
        for attempt in 0..3 {
            self.wait_for_rate_limit().await;
            let mut builder = self.client.get(url).header(USER_AGENT, UA);
            if let Some(key) = &self.api_key {
                builder = builder.header("x-api-key", key);
            }
            let response = builder.send().await.map_err(|err| {
                if err.is_timeout() {
                    GrokSearchError::Timeout(format!("{label} request timed out: {err}"))
                } else {
                    GrokSearchError::Upstream(format!("{label} request failed: {err}"))
                }
            })?;
            let status = response.status();
            let retry_after = retry_after_delay(&response);
            let bytes =
                read_response_bytes_limited(response, label, self.max_response_bytes).await?;
            if !status.is_success() {
                if self.api_key.is_some() && matches!(status.as_u16(), 401 | 403) {
                    return get_json_limited(
                        &self.client,
                        url,
                        &[(USER_AGENT, UA)],
                        label,
                        self.max_response_bytes,
                    )
                    .await;
                }
                if status.as_u16() == 429 && attempt < 2 {
                    last_rate_limit = Some(String::from_utf8_lossy(&bytes).into_owned());
                    tokio::time::sleep(
                        retry_after.unwrap_or_else(|| {
                            Duration::from_millis(2500 + (attempt as u64 * 2500))
                        }),
                    )
                    .await;
                    continue;
                }
                return Err(GrokSearchError::Upstream(format!(
                    "{label} returned HTTP {status}: {}",
                    String::from_utf8_lossy(&bytes)
                )));
            }
            return serde_json::from_slice(&bytes)
                .map_err(|err| GrokSearchError::Parse(format!("invalid {label} JSON: {err}")));
        }
        Err(GrokSearchError::Upstream(format!(
            "{label} returned HTTP 429 after retries: {}",
            last_rate_limit.unwrap_or_else(|| "rate limited".to_string())
        )))
    }

    pub(super) async fn wait_for_rate_limit(&self) {
        const SEMANTIC_MIN_INTERVAL: Duration = Duration::from_millis(1100);

        let mut last_request = self.limiter.lock().await;
        if let Some(last) = *last_request {
            let next = last + SEMANTIC_MIN_INTERVAL;
            let now = Instant::now();
            if next > now {
                sleep_until(next).await;
            }
        }
        *last_request = Some(Instant::now());
        wait_for_global_provider_rate_limit("semantic-scholar", SEMANTIC_MIN_INTERVAL).await;
    }
}

#[async_trait]
impl AcademicProvider for SemanticProvider {
    fn name(&self) -> &'static str {
        "semantic"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://api.semanticscholar.org/graph/v1/paper/search").unwrap();
        url.query_pairs_mut()
            .append_pair("query", &input.query)
            .append_pair("limit", &limit.to_string())
            .append_pair("fields", SEMANTIC_FIELDS);
        if let Some(sort) = semantic_sort(input.sort_by.as_deref()) {
            url.query_pairs_mut().append_pair("sort", sort);
        }
        if let Some(year) = semantic_year_filter(input.year_from, input.year_to) {
            url.query_pairs_mut().append_pair("year", &year);
        }
        if input.open_access_only.unwrap_or(false) {
            url.query_pairs_mut().append_pair("openAccessPdf", "");
        }
        let value = self
            .get_json_with_optional_key(url.as_str(), "semantic scholar")
            .await?;
        Ok(value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_semantic_paper)
            .collect())
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let paper_id = match identifier {
            Identifier::Doi(doi) => format!("DOI:{doi}"),
            Identifier::Arxiv(id) => format!("ARXIV:{id}"),
            Identifier::Semantic(id) => id.clone(),
            _ => return Ok(None),
        };
        let mut url = Url::parse(&format!(
            "https://api.semanticscholar.org/graph/v1/paper/{}",
            paper_id
        ))
        .unwrap();
        url.query_pairs_mut().append_pair("fields", SEMANTIC_FIELDS);
        let value = self
            .get_json_with_optional_key(url.as_str(), "semantic scholar")
            .await?;
        Ok(Some(parse_semantic_paper(&value)))
    }

    async fn citations(
        &self,
        identifier: &Identifier,
        limit: usize,
    ) -> Result<Option<AcademicCitationSummary>> {
        let paper_id = match identifier {
            Identifier::Doi(doi) => format!("DOI:{doi}"),
            Identifier::Arxiv(id) => format!("ARXIV:{id}"),
            Identifier::Semantic(id) => id.clone(),
            _ => return Ok(None),
        };
        let mut citations_url = Url::parse(&format!(
            "https://api.semanticscholar.org/graph/v1/paper/{paper_id}/citations"
        ))
        .unwrap();
        citations_url
            .query_pairs_mut()
            .append_pair("limit", &limit.to_string())
            .append_pair("fields", SEMANTIC_FIELDS);
        let mut refs_url = Url::parse(&format!(
            "https://api.semanticscholar.org/graph/v1/paper/{paper_id}/references"
        ))
        .unwrap();
        refs_url
            .query_pairs_mut()
            .append_pair("limit", &limit.to_string())
            .append_pair("fields", SEMANTIC_FIELDS);
        let citations = self
            .get_json_with_optional_key(citations_url.as_str(), "semantic citations")
            .await?;
        let references = self
            .get_json_with_optional_key(refs_url.as_str(), "semantic references")
            .await?;
        Ok(Some(AcademicCitationSummary {
            citations: semantic_relation_list(&citations, "citingPaper"),
            references: semantic_relation_list(&references, "citedPaper"),
        }))
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let Some(id) = &paper.semantic_scholar_id else {
            return Ok(None);
        };
        let mut url = Url::parse(&format!(
            "https://api.semanticscholar.org/graph/v1/paper/{id}"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("fields", "openAccessPdf,url");
        let value = self
            .get_json_with_optional_key(url.as_str(), "semantic scholar")
            .await?;
        Ok(value
            .pointer("/openAccessPdf/url")
            .and_then(Value::as_str)
            .map(|url| FullTextLocation {
                url: url.to_string(),
                source: "semantic".to_string(),
                status: "open_access_pdf".to_string(),
            }))
    }
}

const SEMANTIC_FIELDS: &str = "paperId,title,authors,year,venue,abstract,url,externalIds,citationCount,referenceCount,openAccessPdf";

pub(crate) fn semantic_year_filter(year_from: Option<u32>, year_to: Option<u32>) -> Option<String> {
    match (year_from, year_to) {
        (Some(from), Some(to)) if from == to => Some(from.to_string()),
        (Some(from), Some(to)) => Some(format!("{from}-{to}")),
        (Some(from), None) => Some(format!("{from}-")),
        (None, Some(to)) => Some(format!("-{to}")),
        (None, None) => None,
    }
}

pub(crate) fn semantic_sort(sort_by: Option<&str>) -> Option<&'static str> {
    if sort_is(sort_by, "citations") {
        Some("citationCount:desc")
    } else if sort_is(sort_by, "date") {
        Some("publicationDate:desc")
    } else {
        None
    }
}

pub(crate) fn parse_semantic_paper(value: &Value) -> AcademicPaper {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let doi = value
        .pointer("/externalIds/DOI")
        .and_then(Value::as_str)
        .map(str::to_string);
    let arxiv_id = value
        .pointer("/externalIds/ArXiv")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = value
        .get("paperId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = value.get("url").and_then(Value::as_str).map(str::to_string);
    let pdf_url = value
        .pointer("/openAccessPdf/url")
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .map(str::to_string);
    let mut paper = AcademicPaper {
        id: doi
            .clone()
            .or_else(|| id.clone())
            .unwrap_or_else(|| title.clone()),
        title,
        authors: value
            .get("authors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a.get("name").and_then(Value::as_str).map(str::to_string))
            .collect(),
        year: value
            .get("year")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        venue: value
            .get("venue")
            .and_then(Value::as_str)
            .map(str::to_string),
        abstract_text: value
            .get("abstract")
            .and_then(Value::as_str)
            .map(str::to_string),
        doi,
        arxiv_id,
        semantic_scholar_id: id,
        url: url.clone(),
        pdf_url,
        citation_count: as_u32(value.get("citationCount").and_then(Value::as_u64)),
        reference_count: as_u32(value.get("referenceCount").and_then(Value::as_u64)),
        open_access: value.get("openAccessPdf").map(|v| !v.is_null()),
        ..Default::default()
    };
    if let Some(url) = url {
        paper
            .sources
            .push(source(url, "semantic", Some(paper.title.clone())));
    }
    paper
}

fn semantic_relation_list(value: &Value, key: &str) -> Vec<AcademicPaper> {
    value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get(key))
        .map(parse_semantic_paper)
        .collect()
}
