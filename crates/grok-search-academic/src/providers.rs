use std::fs::{self, OpenOptions};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use grok_search_net::http::{get_json, get_json_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_net::key_pool::{is_key_scoped_status, KeyPool};
use grok_search_parse::{clean_html_title, extract_arxiv_id_from_path, openalex_abstract};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicPaper, AcademicSearchInput, GrokSearchError, Result,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::header::{RETRY_AFTER, USER_AGENT};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::{sleep_until, Instant};
use url::Url;

use crate::service::{as_u32, source, UA};

#[derive(Clone)]
pub(crate) struct DblpProvider {
    client: reqwest::Client,
}

impl DblpProvider {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AcademicProvider for DblpProvider {
    fn name(&self) -> &'static str {
        "dblp"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://dblp.org/search/publ/api").unwrap();
        url.query_pairs_mut()
            .append_pair("q", &input.query)
            .append_pair("format", "json")
            .append_pair("h", &limit.to_string());
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "dblp").await?;
        Ok(parse_dblp_search(&value))
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        match identifier {
            Identifier::Dblp(key) | Identifier::Url(key) | Identifier::Query(key) => {
                let input = AcademicSearchInput {
                    query: key.clone(),
                    ..Default::default()
                };
                Ok(self.search(&input, 1).await?.into_iter().next())
            }
            _ => Ok(None),
        }
    }
}

pub(crate) fn parse_dblp_search(value: &Value) -> Vec<AcademicPaper> {
    value
        .pointer("/result/hits/hit")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            let info = hit.get("info")?;
            let title = info.get("title").and_then(Value::as_str)?.to_string();
            let url = info.get("url").and_then(Value::as_str).map(str::to_string);
            let doi = info.get("doi").and_then(Value::as_str).map(str::to_string);
            let authors = parse_dblp_authors(info.pointer("/authors/author"));
            let year = info
                .get("year")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok());
            let venue = info
                .get("venue")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut paper = AcademicPaper {
                id: doi
                    .clone()
                    .or_else(|| url.clone())
                    .unwrap_or_else(|| title.clone()),
                title: clean_title(&title),
                authors,
                year,
                venue,
                doi,
                url: url.clone(),
                ..Default::default()
            };
            if let Some(url) = url {
                paper
                    .sources
                    .push(source(url, "dblp", Some(paper.title.clone())));
            }
            Some(paper)
        })
        .collect()
}

fn parse_dblp_authors(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.get("text").or(Some(v)).and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        Some(Value::Object(_)) => value
            .and_then(|v| v.get("text").or(Some(v)).and_then(Value::as_str))
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

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

    async fn get_json_with_optional_key(&self, url: &str, label: &str) -> Result<Value> {
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

    async fn wait_for_rate_limit(&self) {
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

fn retry_after_delay(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

async fn wait_for_global_provider_rate_limit(provider: &str, min_interval: Duration) {
    let base = std::env::temp_dir();
    let stamp_path = base.join(format!("grok-search-rs-{provider}.timestamp"));
    let lock_path = base.join(format!("grok-search-rs-{provider}.lock"));

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_lock) => {
                let _guard = ProviderRateLimitLock { path: lock_path };
                if let Ok(last) = fs::read_to_string(&stamp_path)
                    .ok()
                    .and_then(|raw| raw.trim().parse::<u128>().ok())
                    .ok_or(())
                {
                    let now = unix_millis();
                    let elapsed = now.saturating_sub(last);
                    let min = min_interval.as_millis();
                    if elapsed < min {
                        tokio::time::sleep(Duration::from_millis((min - elapsed) as u64)).await;
                    }
                }
                let _ = fs::write(stamp_path, unix_millis().to_string());
                return;
            }
            Err(_) => {
                if fs::metadata(&lock_path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(10))
                {
                    let _ = fs::remove_file(&lock_path);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

struct ProviderRateLimitLock {
    path: std::path::PathBuf,
}

impl Drop for ProviderRateLimitLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct GetBytesFailure {
    status: Option<u16>,
    retry_after: Option<Duration>,
    error: GrokSearchError,
}

async fn get_bytes_with_status(
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

fn retry_delay(retry_after: Option<Duration>, fallback: Duration) -> Duration {
    retry_after.unwrap_or(fallback)
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

#[derive(Clone)]
pub(crate) struct ArxivProvider {
    client: reqwest::Client,
}

impl ArxivProvider {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn get_text_with_rate_limit(&self, url: &str) -> Result<String> {
        self.get_text_with_rate_limit_interval(url, ARXIV_MIN_INTERVAL)
            .await
    }

    async fn get_text_with_rate_limit_interval(
        &self,
        url: &str,
        min_interval: Duration,
    ) -> Result<String> {
        let mut last_rate_limit = None;
        for attempt in 0..=ARXIV_MAX_RETRIES {
            wait_for_global_provider_rate_limit("arxiv", min_interval).await;
            match get_bytes_with_status(&self.client, url, "arxiv", DEFAULT_MAX_RESPONSE_BYTES)
                .await
            {
                Ok(bytes) => return Ok(String::from_utf8_lossy(&bytes).into_owned()),
                Err(failure) if failure.status == Some(429) && attempt < ARXIV_MAX_RETRIES => {
                    last_rate_limit = Some(failure.error.to_string());
                    tokio::time::sleep(retry_delay(
                        failure.retry_after,
                        Duration::from_secs(3 * (attempt as u64 + 1)),
                    ))
                    .await;
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(GrokSearchError::Upstream(format!(
            "arxiv returned HTTP 429 after retries: {}",
            last_rate_limit.unwrap_or_else(|| "rate limited".to_string())
        )))
    }
}

#[async_trait]
impl AcademicProvider for ArxivProvider {
    fn name(&self) -> &'static str {
        "arxiv"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://export.arxiv.org/api/query").unwrap();
        url.query_pairs_mut()
            .append_pair("search_query", &arxiv_search_query(&input.query))
            .append_pair("start", "0")
            .append_pair("max_results", &limit.to_string())
            .append_pair("sortBy", arxiv_sort_by(input.sort_by.as_deref()))
            .append_pair("sortOrder", "descending");
        let xml = self.get_text_with_rate_limit(url.as_str()).await?;
        parse_arxiv_atom(&xml)
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let Identifier::Arxiv(id) = identifier else {
            return Ok(None);
        };
        let mut url = Url::parse("https://export.arxiv.org/api/query").unwrap();
        url.query_pairs_mut().append_pair("id_list", id);
        let xml = self.get_text_with_rate_limit(url.as_str()).await?;
        Ok(parse_arxiv_atom(&xml)?.into_iter().next())
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let url = paper
            .pdf_url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| paper.arxiv_id.as_deref().map(arxiv_pdf_url));
        Ok(url.map(|url| FullTextLocation {
            url,
            source: "arxiv".to_string(),
            status: "arxiv_pdf".to_string(),
        }))
    }
}

pub(crate) fn arxiv_pdf_url(id: &str) -> String {
    let id = id.trim();
    let id = id.strip_prefix("arXiv:").unwrap_or(id);
    let id = id.strip_suffix(".pdf").unwrap_or(id);
    format!("https://arxiv.org/pdf/{id}")
}

pub(crate) fn arxiv_search_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty()
        || trimmed.contains(':')
        || trimmed.contains('"')
        || trimmed.to_ascii_uppercase().contains(" AND ")
        || trimmed.to_ascii_uppercase().contains(" OR ")
    {
        return trimmed.to_string();
    }
    let terms: Vec<String> = trimmed
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 3 && !ARXIV_QUERY_STOPWORDS.contains(&token.as_str()))
        .take(8)
        .map(|token| format!("all:{token}"))
        .collect();
    if terms.is_empty() {
        trimmed.to_string()
    } else {
        terms.join(" AND ")
    }
}

const ARXIV_QUERY_STOPWORDS: &[&str] = &[
    "a", "all", "an", "and", "are", "for", "from", "how", "into", "is", "not", "of", "on", "or",
    "the", "this", "to", "with", "you",
];

const ARXIV_MIN_INTERVAL: Duration = Duration::from_secs(3);
const ARXIV_MAX_RETRIES: usize = 2;
const OPENALEX_TRANSIENT_MAX_RETRIES: usize = 2;

pub(crate) fn arxiv_sort_by(sort_by: Option<&str>) -> &'static str {
    if sort_is(sort_by, "date") {
        "submittedDate"
    } else {
        "relevance"
    }
}

pub(crate) fn parse_arxiv_atom(xml: &str) -> Result<Vec<AcademicPaper>> {
    #[derive(PartialEq)]
    enum Field {
        None,
        Id,
        Title,
        Summary,
        Name,
        Published,
    }
    let mut reader = Reader::from_str(xml);
    let mut field = Field::None;
    let mut in_entry = false;
    let mut in_author = false;
    let mut buf = String::new();
    let mut papers = Vec::new();
    let mut current = AcademicPaper::default();
    let mut pdf_url = None;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"entry" => {
                    in_entry = true;
                    current = AcademicPaper::default();
                    pdf_url = None;
                }
                b"author" if in_entry => in_author = true,
                b"id" if in_entry => {
                    field = Field::Id;
                    buf.clear();
                }
                b"title" if in_entry => {
                    field = Field::Title;
                    buf.clear();
                }
                b"summary" if in_entry => {
                    field = Field::Summary;
                    buf.clear();
                }
                b"name" if in_author => {
                    field = Field::Name;
                    buf.clear();
                }
                b"published" if in_entry => {
                    field = Field::Published;
                    buf.clear();
                }
                _ => {}
            },
            Ok(Event::Empty(e)) if in_entry && e.name().as_ref() == b"link" => {
                let href = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"href")
                    .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                let typ = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == b"type")
                    .map(|a| String::from_utf8_lossy(&a.value).into_owned());
                if typ.as_deref() == Some("application/pdf") {
                    pdf_url = href;
                }
            }
            Ok(Event::Text(e)) if field != Field::None => {
                buf.push_str(
                    e.unescape()
                        .map_err(|err| {
                            GrokSearchError::Parse(format!("arxiv XML parse error: {err}"))
                        })?
                        .as_ref(),
                );
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"id" if field == Field::Id => {
                    current.url = Some(buf.trim().to_string());
                    current.arxiv_id = current
                        .url
                        .as_deref()
                        .and_then(|u| Url::parse(u).ok())
                        .and_then(|u| extract_arxiv_id_from_path(u.path()));
                    field = Field::None;
                }
                b"title" if field == Field::Title => {
                    current.title = clean_title(buf.trim());
                    field = Field::None;
                }
                b"summary" if field == Field::Summary => {
                    current.abstract_text = Some(buf.trim().to_string());
                    field = Field::None;
                }
                b"name" if field == Field::Name => {
                    current.authors.push(buf.trim().to_string());
                    field = Field::None;
                }
                b"published" if field == Field::Published => {
                    current.year = buf.get(..4).and_then(|s| s.parse().ok());
                    field = Field::None;
                }
                b"author" => in_author = false,
                b"entry" => {
                    in_entry = false;
                    current.pdf_url = pdf_url.clone();
                    current.id = current
                        .arxiv_id
                        .clone()
                        .unwrap_or_else(|| current.title.clone());
                    if let Some(url) = &current.url {
                        current.sources.push(source(
                            url.clone(),
                            "arxiv",
                            Some(current.title.clone()),
                        ));
                    }
                    if !current.title.is_empty() {
                        papers.push(current.clone());
                    }
                }
                _ => {}
            },
            Err(err) => {
                return Err(GrokSearchError::Parse(format!(
                    "arxiv XML parse error: {err}"
                )))
            }
            _ => {}
        }
    }
    Ok(papers)
}

struct GetJsonFailure {
    status: Option<u16>,
    retry_after: Option<Duration>,
    error: GrokSearchError,
}

async fn get_json_with_status(
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

async fn read_response_bytes_limited(
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

#[derive(Clone)]
pub(crate) struct OpenAlexProvider {
    client: reqwest::Client,
    email: Option<String>,
    keys: Option<KeyPool>,
    max_response_bytes: usize,
}

impl OpenAlexProvider {
    #[allow(dead_code)]
    pub(crate) fn new(
        client: reqwest::Client,
        email: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        Self::new_with_limit(client, email, api_key, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub(crate) fn new_with_limit(
        client: reqwest::Client,
        email: Option<String>,
        api_key: Option<String>,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            client,
            email,
            keys: api_key.map(|key| KeyPool::parse(&key)),
            max_response_bytes,
        }
    }

    fn add_mailto(&self, url: &mut Url) {
        if let Some(email) = &self.email {
            url.query_pairs_mut().append_pair("mailto", email);
        }
    }

    fn add_key(&self, url: &mut Url, index: usize) {
        if let Some(keys) = &self.keys {
            url.query_pairs_mut()
                .append_pair("api_key", keys.key(index));
        }
    }

    async fn get_json(&self, base_url: &Url, label: &str) -> Result<Value> {
        let Some(keys) = &self.keys else {
            return self.get_json_with_transient_retries(base_url, label).await;
        };
        let start = keys.start();
        let mut last_key_error = None;
        for offset in 0..keys.len() {
            let mut url = base_url.clone();
            self.add_key(&mut url, start + offset);
            match self.get_json_with_key_retries(&url, label).await {
                Ok(value) => return Ok(value),
                Err(failure) => {
                    if failure.status.is_some_and(is_key_scoped_status) && offset + 1 < keys.len() {
                        last_key_error = Some(failure.error);
                        continue;
                    }
                    return Err(failure.error);
                }
            }
        }
        Err(last_key_error.unwrap_or_else(|| {
            GrokSearchError::Config(format!("{label} request failed with no configured key"))
        }))
    }

    async fn get_json_with_transient_retries(&self, url: &Url, label: &str) -> Result<Value> {
        for attempt in 0..=OPENALEX_TRANSIENT_MAX_RETRIES {
            match get_json_with_status(&self.client, url.as_str(), label, self.max_response_bytes)
                .await
            {
                Ok(value) => return Ok(value),
                Err(failure)
                    if failure.status.is_some_and(is_transient_gateway_status)
                        && attempt < OPENALEX_TRANSIENT_MAX_RETRIES =>
                {
                    tokio::time::sleep(retry_delay(
                        failure.retry_after,
                        openalex_transient_delay(attempt),
                    ))
                    .await;
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(GrokSearchError::Upstream(format!(
            "{label} transient gateway failure after retries"
        )))
    }

    async fn get_json_with_key_retries(
        &self,
        url: &Url,
        label: &str,
    ) -> std::result::Result<Value, GetJsonFailure> {
        for attempt in 0..=OPENALEX_TRANSIENT_MAX_RETRIES {
            match get_json_with_status(&self.client, url.as_str(), label, self.max_response_bytes)
                .await
            {
                Ok(value) => return Ok(value),
                Err(failure)
                    if failure.status.is_some_and(is_transient_gateway_status)
                        && attempt < OPENALEX_TRANSIENT_MAX_RETRIES =>
                {
                    tokio::time::sleep(retry_delay(
                        failure.retry_after,
                        openalex_transient_delay(attempt),
                    ))
                    .await;
                }
                Err(failure) => return Err(failure),
            }
        }
        unreachable!("OpenAlex transient retry loop always returns from match arms")
    }
}

#[async_trait]
impl AcademicProvider for OpenAlexProvider {
    fn name(&self) -> &'static str {
        "openalex"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://api.openalex.org/works").unwrap();
        url.query_pairs_mut()
            .append_pair("search.title_and_abstract", &input.query)
            .append_pair("per-page", &limit.to_string());
        if let Some(filter) = openalex_filter(input) {
            url.query_pairs_mut().append_pair("filter", &filter);
        }
        if let Some(sort) = openalex_sort(input) {
            url.query_pairs_mut().append_pair("sort", sort);
        }
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_openalex_work)
            .map(without_openalex_reference_sources)
            .collect())
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let id = match identifier {
            Identifier::OpenAlex(id) => id.clone(),
            Identifier::Doi(doi) => format!("doi:{doi}"),
            _ => return Ok(None),
        };
        let mut url = Url::parse(&format!("https://api.openalex.org/works/{id}")).unwrap();
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(Some(parse_openalex_work(&value)))
    }

    async fn citations(
        &self,
        identifier: &Identifier,
        limit: usize,
    ) -> Result<Option<AcademicCitationSummary>> {
        let Some(work) = self.get(identifier).await? else {
            return Ok(None);
        };
        let mut summary = AcademicCitationSummary::default();
        if let Some(openalex_id) = &work.openalex_id {
            let mut cited_by = Url::parse("https://api.openalex.org/works").unwrap();
            cited_by
                .query_pairs_mut()
                .append_pair("filter", &format!("cites:{openalex_id}"))
                .append_pair("per-page", &limit.to_string());
            self.add_mailto(&mut cited_by);
            if let Ok(value) = self.get_json(&cited_by, "openalex citations").await {
                summary.citations = value
                    .get("results")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(parse_openalex_work)
                    .collect();
            }
            if let Ok(Some(detail)) = self.get(&Identifier::OpenAlex(openalex_id.clone())).await {
                for referenced in detail
                    .sources
                    .iter()
                    .filter(|s| s.provider.as_ref() == "openalex_reference")
                    .take(limit)
                {
                    summary.references.push(AcademicPaper {
                        id: referenced.url.clone(),
                        title: referenced
                            .title
                            .clone()
                            .unwrap_or_else(|| referenced.url.clone()),
                        openalex_id: Some(referenced.url.clone()),
                        url: Some(referenced.url.clone()),
                        sources: vec![referenced.clone()],
                        ..Default::default()
                    });
                }
            }
        }
        Ok(Some(summary))
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let Some(id) = &paper.openalex_id else {
            return Ok(None);
        };
        let mut url = Url::parse(&format!("https://api.openalex.org/works/{id}")).unwrap();
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(value
            .pointer("/best_oa_location/pdf_url")
            .or_else(|| value.pointer("/primary_location/pdf_url"))
            .and_then(Value::as_str)
            .map(|url| FullTextLocation {
                url: url.to_string(),
                source: "openalex".to_string(),
                status: "openalex_oa_pdf".to_string(),
            }))
    }
}

pub(crate) fn openalex_filter(input: &AcademicSearchInput) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(from) = input.year_from {
        parts.push(format!("from_publication_date:{from}-01-01"));
    }
    if let Some(to) = input.year_to {
        parts.push(format!("to_publication_date:{to}-12-31"));
    }
    if input.open_access_only.unwrap_or(false) {
        parts.push("is_oa:true".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

pub(crate) fn openalex_sort(input: &AcademicSearchInput) -> Option<&'static str> {
    if sort_is(input.sort_by.as_deref(), "citations") {
        Some("cited_by_count:desc")
    } else if sort_is(input.sort_by.as_deref(), "date")
        && (input.year_from.is_some() || input.year_to.is_some())
    {
        Some("publication_date:desc")
    } else {
        None
    }
}

fn is_transient_gateway_status(status: u16) -> bool {
    matches!(status, 502..=504)
}

fn openalex_transient_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        _ => Duration::from_secs(3),
    }
}

pub(crate) fn parse_openalex_work(value: &Value) -> AcademicPaper {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let id = value.get("id").and_then(Value::as_str).map(str::to_string);
    let doi = value
        .get("doi")
        .and_then(Value::as_str)
        .map(|s| s.trim_start_matches("https://doi.org/").to_string());
    let pdf_url = value
        .pointer("/best_oa_location/pdf_url")
        .or_else(|| value.pointer("/primary_location/pdf_url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut paper = AcademicPaper {
        id: doi
            .clone()
            .or_else(|| id.clone())
            .unwrap_or_else(|| title.clone()),
        title,
        authors: value
            .get("authorships")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| {
                a.pointer("/author/display_name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
        year: value
            .get("publication_year")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        venue: value
            .pointer("/primary_location/source/display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        doi,
        openalex_id: id.clone(),
        url: value.get("id").and_then(Value::as_str).map(str::to_string),
        pdf_url,
        citation_count: as_u32(value.get("cited_by_count").and_then(Value::as_u64)),
        open_access: value.pointer("/open_access/is_oa").and_then(Value::as_bool),
        license: value
            .pointer("/best_oa_location/license")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    if let Some(abstract_text) = value.get("abstract_inverted_index") {
        paper.abstract_text = Some(openalex_abstract(abstract_text));
    }
    if let Some(url) = &paper.url {
        paper
            .sources
            .push(source(url.clone(), "openalex", Some(paper.title.clone())));
    }
    if let Some(refs) = value.get("referenced_works").and_then(Value::as_array) {
        paper.reference_count = Some(refs.len() as u32);
        for reference in refs {
            if let Some(url) = reference.as_str() {
                paper
                    .sources
                    .push(source(url.to_string(), "openalex_reference", None));
            }
        }
    }
    paper
}

pub(crate) fn without_openalex_reference_sources(mut paper: AcademicPaper) -> AcademicPaper {
    paper
        .sources
        .retain(|source| source.provider.as_ref() != "openalex_reference");
    paper
}

#[derive(Clone)]
pub(crate) struct CrossrefProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl CrossrefProvider {
    pub(crate) fn new(client: reqwest::Client, email: Option<String>) -> Self {
        Self { client, email }
    }
}

#[async_trait]
impl AcademicProvider for CrossrefProvider {
    fn name(&self) -> &'static str {
        "crossref"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://api.crossref.org/works").unwrap();
        url.query_pairs_mut()
            .append_pair("query.bibliographic", &input.query)
            .append_pair("rows", &limit.to_string());
        if let Some(filter) = crossref_filter(input.year_from, input.year_to) {
            url.query_pairs_mut().append_pair("filter", &filter);
        }
        if let Some((sort, order)) = crossref_sort(input.sort_by.as_deref()) {
            url.query_pairs_mut()
                .append_pair("sort", sort)
                .append_pair("order", order);
        }
        if let Some(email) = &self.email {
            url.query_pairs_mut().append_pair("mailto", email);
        }
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "crossref").await?;
        Ok(value
            .pointer("/message/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_crossref_work)
            .collect())
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let Identifier::Doi(doi) = identifier else {
            return Ok(None);
        };
        let url = format!("https://api.crossref.org/works/{doi}");
        let value = get_json(&self.client, &url, &[(USER_AGENT, UA)], "crossref").await?;
        Ok(value.get("message").map(parse_crossref_work))
    }
}

pub(crate) fn parse_crossref_work(value: &Value) -> AcademicPaper {
    let title = value
        .get("title")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let doi = value.get("DOI").and_then(Value::as_str).map(str::to_string);
    let url = value.get("URL").and_then(Value::as_str).map(str::to_string);
    let mut paper = AcademicPaper {
        id: doi.clone().unwrap_or_else(|| title.clone()),
        title,
        authors: value
            .get("author")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| {
                let given = a.get("given").and_then(Value::as_str).unwrap_or("");
                let family = a.get("family").and_then(Value::as_str).unwrap_or("");
                format!("{given} {family}").trim().to_string()
            })
            .filter(|s| !s.is_empty())
            .collect(),
        year: value
            .pointer("/published-print/date-parts/0/0")
            .or_else(|| value.pointer("/published-online/date-parts/0/0"))
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        venue: value
            .get("container-title")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string),
        abstract_text: value
            .get("abstract")
            .and_then(Value::as_str)
            .map(str::to_string),
        doi,
        url: url.clone(),
        reference_count: as_u32(value.get("reference-count").and_then(Value::as_u64)),
        license: value
            .get("license")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("URL"))
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    if let Some(url) = url {
        paper
            .sources
            .push(source(url, "crossref", Some(paper.title.clone())));
    }
    paper
}

pub(crate) fn crossref_filter(year_from: Option<u32>, year_to: Option<u32>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(from) = year_from {
        parts.push(format!("from-pub-date:{from}-01-01"));
    }
    if let Some(to) = year_to {
        parts.push(format!("until-pub-date:{to}-12-31"));
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

pub(crate) fn crossref_sort(sort_by: Option<&str>) -> Option<(&'static str, &'static str)> {
    if sort_is(sort_by, "citations") {
        Some(("is-referenced-by-count", "desc"))
    } else if sort_is(sort_by, "date") {
        Some(("published", "desc"))
    } else {
        None
    }
}

fn sort_is(sort_by: Option<&str>, expected: &str) -> bool {
    sort_by
        .unwrap_or("relevance")
        .trim()
        .eq_ignore_ascii_case(expected)
}

#[derive(Clone)]
pub(crate) struct UnpaywallProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl UnpaywallProvider {
    pub(crate) fn new(client: reqwest::Client, email: Option<String>) -> Self {
        Self { client, email }
    }
}

#[async_trait]
impl AcademicProvider for UnpaywallProvider {
    fn name(&self) -> &'static str {
        "unpaywall"
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let (Some(email), Some(doi)) = (&self.email, &paper.doi) else {
            return Ok(None);
        };
        let mut url = Url::parse(&format!("https://api.unpaywall.org/v2/{doi}")).unwrap();
        url.query_pairs_mut().append_pair("email", email);
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "unpaywall").await?;
        Ok(value
            .pointer("/best_oa_location/url_for_pdf")
            .or_else(|| value.pointer("/best_oa_location/url"))
            .and_then(Value::as_str)
            .map(|url| FullTextLocation {
                url: url.to_string(),
                source: "unpaywall".to_string(),
                status: "unpaywall_oa_pdf".to_string(),
            }))
    }
}

#[derive(Clone)]
pub(crate) struct SciHubProvider {
    enabled: bool,
    base_url: Option<String>,
}

impl SciHubProvider {
    pub(crate) fn new(_client: reqwest::Client, enabled: bool, base_url: Option<String>) -> Self {
        Self { enabled, base_url }
    }
}

#[async_trait]
impl AcademicProvider for SciHubProvider {
    fn name(&self) -> &'static str {
        "scihub"
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        if !self.enabled {
            return Ok(None);
        }
        let Some(base) = &self.base_url else {
            return Ok(None);
        };
        let Some(identifier) = paper.doi.as_ref().or(paper.arxiv_id.as_ref()) else {
            return Ok(None);
        };
        let base = base.trim_end_matches('/');
        Ok(Some(FullTextLocation {
            url: format!("{base}/{identifier}"),
            source: "scihub".to_string(),
            status: "scihub_fallback_enabled_user_responsibility".to_string(),
        }))
    }
}

fn clean_title(title: &str) -> String {
    clean_html_title(title)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration as StdDuration;

    const ARXIV_OK_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2401.00001v1</id>
    <title>Test Paper</title>
    <summary>Test summary</summary>
    <published>2024-01-01T00:00:00Z</published>
    <author><name>Ada Lovelace</name></author>
    <link href="http://arxiv.org/pdf/2401.00001v1" type="application/pdf"/>
  </entry>
</feed>"#;

    const OPENALEX_OK_BODY: &str = r#"{"id":"https://openalex.org/W1","title":"Test Work","results":[{"id":"https://openalex.org/W1","title":"Test Work"}]}"#;

    struct MockResponse {
        status: u16,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    }

    fn spawn_mock_server(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_log = Arc::clone(&requests);
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buf = [0u8; 4096];
                let read = stream.read(&mut buf).expect("read request");
                request_log
                    .lock()
                    .expect("request log")
                    .push(String::from_utf8_lossy(&buf[..read]).into_owned());
                let reason = match response.status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    502 => "Bad Gateway",
                    503 => "Service Unavailable",
                    504 => "Gateway Timeout",
                    _ => "Status",
                };
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    reason,
                    response.body.len()
                )
                .expect("write status");
                for (name, value) in response.headers {
                    write!(stream, "{name}: {value}\r\n").expect("write header");
                }
                write!(stream, "\r\n{}", response.body).expect("write body");
                stream.flush().expect("flush response");
            }
        });
        (base, requests)
    }

    fn response(status: u16, body: &'static str) -> MockResponse {
        MockResponse {
            status,
            body,
            headers: Vec::new(),
        }
    }

    fn response_with_headers(
        status: u16,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    ) -> MockResponse {
        MockResponse {
            status,
            body,
            headers,
        }
    }

    #[test]
    fn openalex_adds_rotated_api_key_query_parameter() {
        let provider = OpenAlexProvider::new(
            reqwest::Client::new(),
            Some("person@example.com".to_string()),
            Some("oa-a, oa-b".to_string()),
        );
        let mut first = Url::parse("https://api.openalex.org/works").unwrap();
        provider.add_mailto(&mut first);
        provider.add_key(&mut first, 0);
        assert_eq!(
            first.query(),
            Some("mailto=person%40example.com&api_key=oa-a")
        );

        let mut second = Url::parse("https://api.openalex.org/works").unwrap();
        provider.add_key(&mut second, 1);
        assert_eq!(second.query(), Some("api_key=oa-b"));
    }

    #[test]
    fn arxiv_pdf_url_normalizes_common_ids() {
        assert_eq!(
            arxiv_pdf_url("1706.03762"),
            "https://arxiv.org/pdf/1706.03762"
        );
        assert_eq!(
            arxiv_pdf_url("1706.03762v7"),
            "https://arxiv.org/pdf/1706.03762v7"
        );
        assert_eq!(
            arxiv_pdf_url("arXiv:1706.03762.pdf"),
            "https://arxiv.org/pdf/1706.03762"
        );
    }

    #[test]
    fn semantic_year_filter_supports_ranges_and_open_ends() {
        assert_eq!(
            semantic_year_filter(Some(2024), Some(2024)).as_deref(),
            Some("2024")
        );
        assert_eq!(
            semantic_year_filter(Some(2020), Some(2024)).as_deref(),
            Some("2020-2024")
        );
        assert_eq!(
            semantic_year_filter(Some(2020), None).as_deref(),
            Some("2020-")
        );
        assert_eq!(
            semantic_year_filter(None, Some(2024)).as_deref(),
            Some("-2024")
        );
        assert!(semantic_year_filter(None, None).is_none());
    }

    #[test]
    fn provider_sort_params_map_common_academic_preferences() {
        assert_eq!(semantic_sort(Some("citations")), Some("citationCount:desc"));
        assert_eq!(semantic_sort(Some("date")), Some("publicationDate:desc"));
        assert_eq!(semantic_sort(Some("relevance")), None);
        assert_eq!(arxiv_sort_by(Some("date")), "submittedDate");
        assert_eq!(arxiv_sort_by(Some("citations")), "relevance");
        let openalex_citations = AcademicSearchInput {
            sort_by: Some("citations".to_string()),
            ..Default::default()
        };
        assert_eq!(
            openalex_sort(&openalex_citations),
            Some("cited_by_count:desc")
        );
        let openalex_broad_date = AcademicSearchInput {
            sort_by: Some("date".to_string()),
            ..Default::default()
        };
        assert_eq!(openalex_sort(&openalex_broad_date), None);
        let openalex_filtered_date = AcademicSearchInput {
            sort_by: Some("date".to_string()),
            year_from: Some(2024),
            ..Default::default()
        };
        assert_eq!(
            openalex_sort(&openalex_filtered_date),
            Some("publication_date:desc")
        );
        assert_eq!(
            crossref_sort(Some("citations")),
            Some(("is-referenced-by-count", "desc"))
        );
        assert_eq!(crossref_sort(Some("date")), Some(("published", "desc")));
    }

    #[test]
    fn arxiv_search_query_rewrites_plain_text_to_all_terms() {
        assert_eq!(
            arxiv_search_query("large language model evaluation"),
            "all:large AND all:language AND all:model AND all:evaluation"
        );
        assert_eq!(
            arxiv_search_query("Attention Is All You Need"),
            "all:attention AND all:need"
        );
        assert_eq!(arxiv_search_query("ti:transformer"), "ti:transformer");
    }

    #[test]
    fn openalex_filter_includes_dates_and_open_access() {
        let mut input = AcademicSearchInput {
            year_from: Some(2024),
            year_to: Some(2025),
            open_access_only: Some(true),
            ..Default::default()
        };
        assert_eq!(
            openalex_filter(&input).as_deref(),
            Some("from_publication_date:2024-01-01,to_publication_date:2025-12-31,is_oa:true")
        );
        input.year_from = None;
        assert_eq!(
            openalex_filter(&input).as_deref(),
            Some("to_publication_date:2025-12-31,is_oa:true")
        );
        assert!(openalex_filter(&AcademicSearchInput::default()).is_none());
    }

    #[test]
    fn crossref_filter_uses_publication_date_bounds() {
        assert_eq!(
            crossref_filter(Some(2024), Some(2025)).as_deref(),
            Some("from-pub-date:2024-01-01,until-pub-date:2025-12-31")
        );
        assert_eq!(
            crossref_filter(None, Some(2025)).as_deref(),
            Some("until-pub-date:2025-12-31")
        );
        assert!(crossref_filter(None, None).is_none());
    }

    #[tokio::test]
    async fn arxiv_429_retries_and_returns_xml() {
        let (base, requests) = spawn_mock_server(vec![
            response_with_headers(429, "rate limited", vec![("Retry-After", "0")]),
            response(200, ARXIV_OK_BODY),
        ]);
        let provider = ArxivProvider::new(reqwest::Client::builder().build().unwrap());
        let papers = provider
            .get_text_with_rate_limit_interval(
                &format!("{base}/api/query?id_list=2401.00001"),
                Duration::ZERO,
            )
            .await
            .and_then(|xml| parse_arxiv_atom(&xml))
            .expect("arxiv retry should succeed");

        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].arxiv_id.as_deref(), Some("2401.00001v1"));
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn arxiv_429_after_retries_keeps_status_in_error() {
        let (base, requests) = spawn_mock_server(vec![
            response_with_headers(429, "rate limited", vec![("Retry-After", "0")]),
            response_with_headers(429, "still limited", vec![("Retry-After", "0")]),
            response_with_headers(429, "still limited", vec![("Retry-After", "0")]),
        ]);
        let provider = ArxivProvider::new(reqwest::Client::builder().build().unwrap());
        let err = provider
            .get_text_with_rate_limit_interval(
                &format!("{base}/api/query?id_list=2401.00001"),
                Duration::ZERO,
            )
            .await
            .expect_err("429 should remain visible after retries");

        assert!(err.to_string().contains("HTTP 429"), "{err}");
        assert_eq!(requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn openalex_504_retries_without_key_rotation() {
        let (base, requests) = spawn_mock_server(vec![
            response(504, r#"{"error":"Gateway timeout"}"#),
            response(200, OPENALEX_OK_BODY),
        ]);
        let provider = OpenAlexProvider::new_with_limit(
            reqwest::Client::new(),
            None,
            Some("oa-a,oa-b".to_string()),
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let url = Url::parse(&format!("{base}/works/W1")).unwrap();
        let value = provider.get_json(&url, "openalex").await.unwrap();

        assert_eq!(value["id"], "https://openalex.org/W1");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.contains("api_key=oa-a")),
            "{requests:?}"
        );
    }

    #[tokio::test]
    async fn openalex_429_with_keys_rotates_without_transient_retry() {
        let (base, requests) = spawn_mock_server(vec![
            response(429, r#"{"error":"rate limited"}"#),
            response(200, OPENALEX_OK_BODY),
        ]);
        let provider = OpenAlexProvider::new_with_limit(
            reqwest::Client::new(),
            None,
            Some("oa-a,oa-b".to_string()),
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let url = Url::parse(&format!("{base}/works/W1")).unwrap();
        let value = provider.get_json(&url, "openalex").await.unwrap();

        assert_eq!(value["id"], "https://openalex.org/W1");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("api_key=oa-a"), "{requests:?}");
        assert!(requests[1].contains("api_key=oa-b"), "{requests:?}");
    }

    #[tokio::test]
    async fn semantic_request_sends_api_key_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}/paper", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).expect("read request");
            let request = String::from_utf8_lossy(&buf[..read]).into_owned();
            let body = r#"{"data":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write response");
            request
        });

        let provider = SemanticProvider::new(reqwest::Client::new(), Some("s2-test".into()));
        provider
            .get_json_with_optional_key(&url, "semantic scholar")
            .await
            .expect("mock semantic response");
        let request = handle.join().expect("server thread");
        assert!(
            request.to_ascii_lowercase().contains("x-api-key: s2-test"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn semantic_rate_limit_serializes_consecutive_requests() {
        let provider = SemanticProvider::new(reqwest::Client::new(), Some("s2-test".into()));
        let start = Instant::now();
        provider.wait_for_rate_limit().await;
        provider.wait_for_rate_limit().await;
        assert!(
            start.elapsed() >= StdDuration::from_millis(1000),
            "second S2 request should be delayed below the 1 rps threshold"
        );
    }

    #[tokio::test]
    async fn provider_rate_limit_serializes_consecutive_requests() {
        let provider = format!("test-provider-{}", unix_millis());
        let start = Instant::now();
        wait_for_global_provider_rate_limit(&provider, Duration::from_millis(80)).await;
        wait_for_global_provider_rate_limit(&provider, Duration::from_millis(80)).await;
        assert!(
            start.elapsed() >= StdDuration::from_millis(70),
            "second provider request should be delayed by the global limiter"
        );
    }
}
