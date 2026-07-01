use std::time::Duration;

use async_trait::async_trait;
use grok_search_net::http::DEFAULT_MAX_RESPONSE_BYTES;
use grok_search_parse::extract_arxiv_id_from_path;
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
use grok_search_types::{AcademicPaper, AcademicSearchInput, GrokSearchError, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use url::Url;

use super::clean_title;
use super::http::{get_bytes_with_status, retry_delay};
use super::rate_limit::wait_for_global_provider_rate_limit;
use super::sort_is;
use crate::service::source;

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

    pub(super) async fn get_text_with_rate_limit_interval(
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
