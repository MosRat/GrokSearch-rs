use std::collections::{BTreeMap, HashMap};
use std::io::Write;

use async_trait::async_trait;
use grok_search_config::Config;
use grok_search_net::http::{get_bytes, get_json, get_text};
use grok_search_types::{
    AcademicCitationSummary, AcademicCitationsOutput, AcademicGetOutput, AcademicPaper,
    AcademicReadOutput, AcademicSearchInput, AcademicSearchOutput, GrokSearchError, Result, Source,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde_json::Value;
use url::Url;
use uuid::Uuid;

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";
const DEFAULT_SOURCES: &[&str] = &["dblp", "semantic", "arxiv"];
const ALL_SOURCES: &[&str] = &["dblp", "semantic", "arxiv", "openalex", "crossref"];

#[async_trait]
pub trait AcademicProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(
        &self,
        _query: &AcademicSearchInput,
        _limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        Err(GrokSearchError::Provider(format!(
            "{} does not support academic search",
            self.name()
        )))
    }
    async fn get(&self, _identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        Ok(None)
    }
    async fn citations(
        &self,
        _identifier: &Identifier,
        _limit: usize,
    ) -> Result<Option<AcademicCitationSummary>> {
        Ok(None)
    }
    async fn resolve_fulltext(&self, _paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        Ok(None)
    }
}

#[derive(Clone)]
pub struct AcademicService {
    client: reqwest::Client,
    config: Config,
    providers: ProviderSet,
}

#[derive(Clone)]
struct ProviderSet {
    dblp: DblpProvider,
    semantic: SemanticProvider,
    arxiv: ArxivProvider,
    openalex: OpenAlexProvider,
    crossref: CrossrefProvider,
    unpaywall: UnpaywallProvider,
    scihub: SciHubProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identifier {
    Doi(String),
    Arxiv(String),
    Semantic(String),
    OpenAlex(String),
    Dblp(String),
    Url(String),
    Query(String),
}

#[derive(Debug, Clone)]
pub struct FullTextLocation {
    pub url: String,
    pub source: String,
    pub status: String,
}

impl AcademicService {
    pub fn new(client: reqwest::Client, config: Config) -> Self {
        Self {
            providers: ProviderSet {
                dblp: DblpProvider::new(client.clone()),
                semantic: SemanticProvider::new(
                    client.clone(),
                    config.semantic_scholar_api_key.clone(),
                ),
                arxiv: ArxivProvider::new(client.clone()),
                openalex: OpenAlexProvider::new(client.clone(), config.academic_email.clone()),
                crossref: CrossrefProvider::new(client.clone(), config.academic_email.clone()),
                unpaywall: UnpaywallProvider::new(client.clone(), config.academic_email.clone()),
                scihub: SciHubProvider::new(
                    client.clone(),
                    config.academic_scihub_enabled,
                    config.academic_scihub_base_url.clone(),
                ),
            },
            client,
            config,
        }
    }

    pub async fn search(&self, input: AcademicSearchInput) -> Result<AcademicSearchOutput> {
        if input.query.trim().is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "academic_search.query is required".to_string(),
            ));
        }
        let limit = input.max_results.unwrap_or(10).clamp(1, 50);
        let selected = selected_sources(&input.sources);
        let mut errors = BTreeMap::new();
        let mut ranked: Vec<(String, Vec<AcademicPaper>)> = Vec::new();

        for source in selected {
            let result = match source.as_str() {
                "dblp" => self.providers.dblp.search(&input, limit).await,
                "semantic" => self.providers.semantic.search(&input, limit).await,
                "arxiv" => self.providers.arxiv.search(&input, limit).await,
                "openalex" => self.providers.openalex.search(&input, limit).await,
                "crossref" => self.providers.crossref.search(&input, limit).await,
                other => Err(GrokSearchError::InvalidParams(format!(
                    "unknown academic source: {other}"
                ))),
            };
            match result {
                Ok(papers) => ranked.push((source, papers)),
                Err(err) => {
                    errors.insert(source, err.to_string());
                }
            }
        }

        let sources_used = ranked.iter().map(|(name, _)| name.clone()).collect();
        let mut papers = rrf_merge(ranked);
        if input.open_access_only.unwrap_or(false) {
            papers.retain(|paper| paper.open_access.unwrap_or(false) || paper.pdf_url.is_some());
        }
        if input.include_abstract == Some(false) {
            for paper in &mut papers {
                paper.abstract_text = None;
            }
        }
        papers.truncate(limit);

        if input.include_citations.unwrap_or(false) {
            for paper in &mut papers {
                if paper.citation_count.is_none() || paper.reference_count.is_none() {
                    if let Ok(Some(summary)) = self
                        .providers
                        .semantic
                        .citations(&identifier_for_paper(paper), 1)
                        .await
                    {
                        paper.citation_count = paper
                            .citation_count
                            .or(Some(summary.citations.len() as u32));
                        paper.reference_count = paper
                            .reference_count
                            .or(Some(summary.references.len() as u32));
                    }
                }
            }
        }

        Ok(AcademicSearchOutput {
            session_id: short_session_id(),
            papers_count: papers.len(),
            papers,
            sources_used,
            errors,
            truncated: false,
        })
    }

    pub async fn get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
    ) -> Result<AcademicGetOutput> {
        let id = parse_identifier(identifier);
        let mut chain = Vec::new();
        let mut paper: Option<AcademicPaper> = None;
        for provider in self.get_providers() {
            chain.push(provider.name().to_string());
            match provider.get(&id).await {
                Ok(Some(found)) => {
                    paper = Some(match paper {
                        Some(mut existing) => {
                            existing.merge_from(found);
                            existing
                        }
                        None => found,
                    });
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }
        let mut paper = paper.ok_or_else(|| {
            GrokSearchError::NotFound(format!("academic identifier not found: {identifier}"))
        })?;
        if include_open_access {
            if let Ok(Some(location)) = self.resolve_fulltext_location(&paper).await {
                paper.pdf_url = paper.pdf_url.or(Some(location.url));
            }
        }
        let citations = if include_citations {
            self.citation_summary(&identifier_for_paper(&paper), 10)
                .await
                .ok()
        } else {
            None
        };
        Ok(AcademicGetOutput {
            paper,
            citations,
            resolver_chain: chain,
        })
    }

    pub async fn citations(
        &self,
        identifier: &str,
        limit: usize,
    ) -> Result<AcademicCitationsOutput> {
        let id = parse_identifier(identifier);
        let limit = limit.clamp(1, 50);
        let mut sources_used = Vec::new();
        for provider in [
            &self.providers.semantic as &dyn AcademicProvider,
            &self.providers.openalex,
        ] {
            match provider.citations(&id, limit).await {
                Ok(Some(summary)) => {
                    sources_used.push(provider.name().to_string());
                    return Ok(AcademicCitationsOutput {
                        identifier: identifier.to_string(),
                        citation_count: Some(summary.citations.len() as u32),
                        reference_count: Some(summary.references.len() as u32),
                        citations: summary.citations,
                        references: summary.references,
                        sources_used,
                    });
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }
        Err(GrokSearchError::NotFound(format!(
            "citations unavailable for {identifier}"
        )))
    }

    pub async fn read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
    ) -> Result<AcademicReadOutput> {
        let format = output_format.unwrap_or_else(|| "markdown".to_string());
        if format != "markdown" && format != "text" {
            return Err(GrokSearchError::InvalidParams(
                "output_format must be \"markdown\" or \"text\"".to_string(),
            ));
        }
        let mut chain = Vec::new();
        let location = if let Some(url) = url.clone() {
            FullTextLocation {
                url,
                source: "direct_url".to_string(),
                status: "direct_url".to_string(),
            }
        } else {
            let identifier = identifier.as_deref().ok_or_else(|| {
                GrokSearchError::InvalidParams("academic_read requires identifier or url".into())
            })?;
            let get = self.get(identifier, false, true).await?;
            chain.extend(get.resolver_chain);
            self.resolve_fulltext_location(&get.paper)
                .await?
                .ok_or_else(|| GrokSearchError::NotFound("no full-text PDF URL found".into()))?
        };

        chain.push(location.source.clone());
        let bytes = download_pdf_bytes(
            &self.client,
            &location.url,
            self.config.academic_max_pdf_bytes,
        )
        .await?;
        let limit = max_chars
            .or(self.config.academic_pdf_max_chars)
            .or(self.config.fetch_max_chars)
            .unwrap_or(200_000);
        let parsed = pdf::parse_pdf_bytes(&bytes, &format, Some(limit))?;
        Ok(AcademicReadOutput {
            identifier,
            url,
            pdf_url: location.url,
            content: parsed.content,
            original_length: parsed.original_length,
            truncated: parsed.truncated,
            source: location.source,
            fulltext_status: location.status,
            resolver_chain: chain,
        })
    }

    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.config.academic_enabled,
            "semantic_scholar_api_key": self.config.semantic_scholar_key_status(),
            "unpaywall_email": self.config.academic_email_status(),
            "scihub_enabled": self.config.academic_scihub_enabled,
            "scihub_base_url": self.config.redacted_scihub_base_url(),
            "pdf_parser": "pdf_oxide",
            "providers": ["dblp", "semantic", "arxiv", "openalex", "crossref", "unpaywall"],
        })
    }

    async fn citation_summary(
        &self,
        id: &Identifier,
        limit: usize,
    ) -> Result<AcademicCitationSummary> {
        if let Ok(Some(summary)) = self.providers.semantic.citations(id, limit).await {
            return Ok(summary);
        }
        if let Ok(Some(summary)) = self.providers.openalex.citations(id, limit).await {
            return Ok(summary);
        }
        Err(GrokSearchError::NotFound(
            "citation summary unavailable".into(),
        ))
    }

    async fn resolve_fulltext_location(
        &self,
        paper: &AcademicPaper,
    ) -> Result<Option<FullTextLocation>> {
        for provider in [
            &self.providers.arxiv as &dyn AcademicProvider,
            &self.providers.semantic,
            &self.providers.openalex,
            &self.providers.unpaywall,
            &self.providers.scihub,
        ] {
            if let Some(location) = provider.resolve_fulltext(paper).await? {
                return Ok(Some(location));
            }
        }
        Ok(None)
    }

    fn get_providers(&self) -> Vec<&dyn AcademicProvider> {
        vec![
            &self.providers.dblp,
            &self.providers.semantic,
            &self.providers.arxiv,
            &self.providers.openalex,
            &self.providers.crossref,
        ]
    }
}

fn selected_sources(raw: &[String]) -> Vec<String> {
    let requested: Vec<String> = if raw.is_empty() {
        DEFAULT_SOURCES.iter().map(|s| s.to_string()).collect()
    } else {
        raw.iter()
            .flat_map(|s| s.split(','))
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    };
    requested
        .into_iter()
        .filter(|source| ALL_SOURCES.contains(&source.as_str()))
        .collect()
}

fn short_session_id() -> String {
    let mut uuid_buf = [0u8; uuid::fmt::Simple::LENGTH];
    Uuid::new_v4().simple().encode_lower(&mut uuid_buf)[..12].to_string()
}

fn paper_key(paper: &AcademicPaper) -> String {
    if let Some(doi) = &paper.doi {
        return format!("doi:{}", doi.to_ascii_lowercase());
    }
    if let Some(arxiv) = &paper.arxiv_id {
        return format!("arxiv:{}", arxiv.to_ascii_lowercase());
    }
    if let Some(id) = &paper.semantic_scholar_id {
        return format!("semantic:{}", id.to_ascii_lowercase());
    }
    if let Some(id) = &paper.openalex_id {
        return format!("openalex:{}", id.to_ascii_lowercase());
    }
    format!(
        "title:{}:{}",
        normalize_title(&paper.title),
        paper.year.map(|y| y.to_string()).unwrap_or_default()
    )
}

fn rrf_merge(ranked: Vec<(String, Vec<AcademicPaper>)>) -> Vec<AcademicPaper> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut papers: HashMap<String, AcademicPaper> = HashMap::new();
    for (_source, list) in ranked {
        for (idx, paper) in list.into_iter().enumerate() {
            let key = paper_key(&paper);
            *scores.entry(key.clone()).or_default() += 1.0 / (60.0 + idx as f64 + 1.0);
            papers
                .entry(key)
                .and_modify(|existing| existing.merge_from(paper.clone()))
                .or_insert(paper);
        }
    }
    let mut items: Vec<_> = papers.into_iter().collect();
    items.sort_by(|(a, _), (b, _)| {
        scores
            .get(b)
            .partial_cmp(&scores.get(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.into_iter().map(|(_, paper)| paper).collect()
}

fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn parse_identifier(raw: &str) -> Identifier {
    let value = raw.trim();
    if value.starts_with("10.") || value.to_ascii_lowercase().starts_with("doi:10.") {
        return Identifier::Doi(value.trim_start_matches("doi:").to_string());
    }
    if let Ok(url) = Url::parse(value) {
        let host = url.host_str().unwrap_or_default();
        if host.ends_with("arxiv.org") {
            if let Some(id) = extract_arxiv_id_from_path(url.path()) {
                return Identifier::Arxiv(id);
            }
        }
        if host.ends_with("openalex.org") {
            return Identifier::OpenAlex(value.to_string());
        }
        if host.ends_with("dblp.org") {
            return Identifier::Dblp(value.to_string());
        }
        return Identifier::Url(value.to_string());
    }
    if value.starts_with("arXiv:") || looks_like_arxiv_id(value) {
        return Identifier::Arxiv(value.trim_start_matches("arXiv:").to_string());
    }
    if value.starts_with("W") && value[1..].chars().all(|c| c.is_ascii_digit()) {
        return Identifier::OpenAlex(value.to_string());
    }
    if value.len() >= 32 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Identifier::Semantic(value.to_string());
    }
    Identifier::Query(value.to_string())
}

fn identifier_for_paper(paper: &AcademicPaper) -> Identifier {
    paper
        .doi
        .as_ref()
        .map(|v| Identifier::Doi(v.clone()))
        .or_else(|| {
            paper
                .arxiv_id
                .as_ref()
                .map(|v| Identifier::Arxiv(v.clone()))
        })
        .or_else(|| {
            paper
                .semantic_scholar_id
                .as_ref()
                .map(|v| Identifier::Semantic(v.clone()))
        })
        .or_else(|| {
            paper
                .openalex_id
                .as_ref()
                .map(|v| Identifier::OpenAlex(v.clone()))
        })
        .unwrap_or_else(|| Identifier::Query(paper.title.clone()))
}

fn looks_like_arxiv_id(value: &str) -> bool {
    let value = value.strip_suffix(".pdf").unwrap_or(value);
    let mut parts = value.split('.');
    matches!((parts.next(), parts.next()), (Some(a), Some(b)) if a.len() == 4 && b.len() >= 4 && a.chars().all(|c| c.is_ascii_digit()))
}

fn extract_arxiv_id_from_path(path: &str) -> Option<String> {
    for prefix in ["/abs/", "/pdf/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return Some(rest.strip_suffix(".pdf").unwrap_or(rest).to_string());
        }
    }
    None
}

fn source(url: impl Into<String>, provider: &'static str, title: Option<String>) -> Source {
    let mut source = Source::new(url, provider);
    source.title = title;
    source
}

fn as_u32(value: Option<u64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

async fn download_pdf_bytes(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let bytes = get_bytes(
        client,
        url,
        &[(USER_AGENT, UA), (ACCEPT, "application/pdf")],
        "academic pdf",
    )
    .await?;
    if bytes.len() > max_bytes {
        return Err(GrokSearchError::Provider(format!(
            "academic pdf exceeds max size: {} > {}",
            bytes.len(),
            max_bytes
        )));
    }
    if !bytes.starts_with(b"%PDF") {
        return Err(GrokSearchError::Provider(
            "resolved academic full text is not a PDF".to_string(),
        ));
    }
    Ok(bytes)
}

pub mod pdf {
    use super::*;

    pub struct ParsedPdf {
        pub content: String,
        pub original_length: usize,
        pub truncated: bool,
    }

    pub fn parse_pdf_bytes(
        bytes: &[u8],
        format: &str,
        max_chars: Option<usize>,
    ) -> Result<ParsedPdf> {
        let mut file = tempfile::NamedTempFile::new()
            .map_err(|err| GrokSearchError::Provider(format!("create temp PDF: {err}")))?;
        file.write_all(bytes)
            .map_err(|err| GrokSearchError::Provider(format!("write temp PDF: {err}")))?;
        let path = file.path().to_path_buf();
        let content = parse_with_pdf_oxide(&path, format)?;
        let original_length = content.chars().count();
        let mut truncated = false;
        let content = if let Some(limit) = max_chars {
            if original_length > limit {
                truncated = true;
                content.chars().take(limit).collect()
            } else {
                content
            }
        } else {
            content
        };
        Ok(ParsedPdf {
            content,
            original_length,
            truncated,
        })
    }

    fn parse_with_pdf_oxide(path: &std::path::Path, format: &str) -> Result<String> {
        let doc = pdf_oxide::PdfDocument::open(path)
            .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide open: {err}")))?;
        let pages = doc
            .page_count()
            .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide page_count: {err}")))?;
        let mut out = String::new();
        for page in 0..pages {
            let text = if format == "markdown" {
                doc.to_markdown(page, &pdf_oxide::converters::ConversionOptions::default())
            } else {
                doc.extract_text(page)
            }
            .map_err(|err| {
                GrokSearchError::Parse(format!("pdf_oxide extract page {page}: {err}"))
            })?;
            out.push_str(&text);
            out.push_str("\n\n");
        }
        Ok(out)
    }
}

#[derive(Clone)]
struct DblpProvider {
    client: reqwest::Client,
}

impl DblpProvider {
    fn new(client: reqwest::Client) -> Self {
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

fn parse_dblp_search(value: &Value) -> Vec<AcademicPaper> {
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
struct SemanticProvider {
    client: reqwest::Client,
    api_key: Option<String>,
}

impl SemanticProvider {
    fn new(client: reqwest::Client, api_key: Option<String>) -> Self {
        Self { client, api_key }
    }

    async fn get_json_with_optional_key(&self, url: &str, label: &str) -> Result<Value> {
        let mut builder = self.client.get(url).header(USER_AGENT, UA);
        if let Some(key) = &self.api_key {
            builder = builder.header("x-api-key", key);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| GrokSearchError::Provider(format!("{label} request failed: {err}")))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|err| GrokSearchError::Provider(format!("{label} body read failed: {err}")))?;
        if !status.is_success() {
            if self.api_key.is_some() && matches!(status.as_u16(), 401 | 403) {
                return get_json(&self.client, url, &[(USER_AGENT, UA)], label).await;
            }
            return Err(GrokSearchError::Provider(format!(
                "{label} returned HTTP {status}: {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        serde_json::from_slice(&bytes)
            .map_err(|err| GrokSearchError::Parse(format!("invalid {label} JSON: {err}")))
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

fn parse_semantic_paper(value: &Value) -> AcademicPaper {
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
struct ArxivProvider {
    client: reqwest::Client,
}

impl ArxivProvider {
    fn new(client: reqwest::Client) -> Self {
        Self { client }
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
            .append_pair("search_query", &input.query)
            .append_pair("start", "0")
            .append_pair("max_results", &limit.to_string())
            .append_pair("sortBy", "relevance")
            .append_pair("sortOrder", "descending");
        let xml = get_text(&self.client, url.as_str(), &[(USER_AGENT, UA)], "arxiv").await?;
        parse_arxiv_atom(&xml)
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let Identifier::Arxiv(id) = identifier else {
            return Ok(None);
        };
        let mut url = Url::parse("https://export.arxiv.org/api/query").unwrap();
        url.query_pairs_mut().append_pair("id_list", id);
        let xml = get_text(&self.client, url.as_str(), &[(USER_AGENT, UA)], "arxiv").await?;
        Ok(parse_arxiv_atom(&xml)?.into_iter().next())
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        Ok(paper.arxiv_id.as_ref().map(|id| FullTextLocation {
            url: format!("https://arxiv.org/pdf/{id}.pdf"),
            source: "arxiv".to_string(),
            status: "arxiv_pdf".to_string(),
        }))
    }
}

fn parse_arxiv_atom(xml: &str) -> Result<Vec<AcademicPaper>> {
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

#[derive(Clone)]
struct OpenAlexProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl OpenAlexProvider {
    fn new(client: reqwest::Client, email: Option<String>) -> Self {
        Self { client, email }
    }

    fn add_mailto(&self, url: &mut Url) {
        if let Some(email) = &self.email {
            url.query_pairs_mut().append_pair("mailto", email);
        }
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
            .append_pair("search", &input.query)
            .append_pair("per-page", &limit.to_string());
        if let Some(from) = input.year_from {
            url.query_pairs_mut()
                .append_pair("filter", &format!("from_publication_date:{from}-01-01"));
        }
        self.add_mailto(&mut url);
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "openalex").await?;
        Ok(value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_openalex_work)
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
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "openalex").await?;
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
            if let Ok(value) = get_json(
                &self.client,
                cited_by.as_str(),
                &[(USER_AGENT, UA)],
                "openalex citations",
            )
            .await
            {
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
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "openalex").await?;
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

fn parse_openalex_work(value: &Value) -> AcademicPaper {
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

fn openalex_abstract(value: &Value) -> String {
    let Some(map) = value.as_object() else {
        return String::new();
    };
    let mut words: Vec<(usize, &str)> = Vec::new();
    for (word, positions) in map {
        if let Some(items) = positions.as_array() {
            for pos in items {
                if let Some(pos) = pos.as_u64() {
                    words.push((pos as usize, word));
                }
            }
        }
    }
    words.sort_by_key(|(pos, _)| *pos);
    words
        .into_iter()
        .map(|(_, word)| word)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone)]
struct CrossrefProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl CrossrefProvider {
    fn new(client: reqwest::Client, email: Option<String>) -> Self {
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

fn parse_crossref_work(value: &Value) -> AcademicPaper {
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

#[derive(Clone)]
struct UnpaywallProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl UnpaywallProvider {
    fn new(client: reqwest::Client, email: Option<String>) -> Self {
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
struct SciHubProvider {
    enabled: bool,
    base_url: Option<String>,
}

impl SciHubProvider {
    fn new(_client: reqwest::Client, enabled: bool, base_url: Option<String>) -> Self {
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
    title
        .replace("<sub>", "")
        .replace("</sub>", "")
        .replace("<sup>", "")
        .replace("</sup>", "")
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identifier_normalizes_common_academic_ids() {
        assert_eq!(
            parse_identifier("https://arxiv.org/pdf/1706.03762.pdf"),
            Identifier::Arxiv("1706.03762".to_string())
        );
        assert_eq!(
            parse_identifier("10.1145/3368089.3409742"),
            Identifier::Doi("10.1145/3368089.3409742".to_string())
        );
        assert_eq!(
            parse_identifier("https://openalex.org/W2741809807"),
            Identifier::OpenAlex("https://openalex.org/W2741809807".to_string())
        );
    }

    #[test]
    fn dblp_fixture_parses_core_metadata() {
        let value = json!({
            "result": { "hits": { "hit": [{
                "info": {
                    "title": "Attention Is All You Need",
                    "authors": { "author": [{ "text": "Ashish Vaswani" }, { "text": "Noam Shazeer" }] },
                    "year": "2017",
                    "venue": "NIPS",
                    "doi": "10.5555/3295222.3295349",
                    "url": "https://dblp.org/rec/conf/nips/VaswaniSPUJGKP17"
                }
            }] } }
        });
        let papers = parse_dblp_search(&value);
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Attention Is All You Need");
        assert_eq!(papers[0].authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(papers[0].doi.as_deref(), Some("10.5555/3295222.3295349"));
        assert_eq!(papers[0].sources[0].provider.as_ref(), "dblp");
    }

    #[test]
    fn semantic_fixture_parses_ids_and_counts() {
        let value = json!({
            "paperId": "abc123",
            "title": "A Paper",
            "authors": [{ "name": "Ada Lovelace" }],
            "year": 2024,
            "venue": "SOSP",
            "abstract": "Abstract",
            "url": "https://semanticscholar.org/paper/abc123",
            "externalIds": { "DOI": "10.1/example", "ArXiv": "2401.00001" },
            "citationCount": 7,
            "referenceCount": 3,
            "openAccessPdf": { "url": "https://example.com/paper.pdf" }
        });
        let paper = parse_semantic_paper(&value);
        assert_eq!(paper.semantic_scholar_id.as_deref(), Some("abc123"));
        assert_eq!(paper.arxiv_id.as_deref(), Some("2401.00001"));
        assert_eq!(paper.citation_count, Some(7));
        assert_eq!(paper.open_access, Some(true));
    }

    #[test]
    fn openalex_inverted_abstract_is_reconstructed() {
        let value = json!({
            "id": "https://openalex.org/W1",
            "title": "Open Work",
            "publication_year": 2025,
            "authorships": [{ "author": { "display_name": "Grace Hopper" } }],
            "abstract_inverted_index": { "hello": [0], "world": [1] },
            "cited_by_count": 42,
            "referenced_works": ["https://openalex.org/W0"],
            "open_access": { "is_oa": true },
            "best_oa_location": { "pdf_url": "https://example.com/oa.pdf", "license": "cc-by" }
        });
        let paper = parse_openalex_work(&value);
        assert_eq!(paper.abstract_text.as_deref(), Some("hello world"));
        assert_eq!(paper.citation_count, Some(42));
        assert_eq!(paper.reference_count, Some(1));
        assert_eq!(paper.pdf_url.as_deref(), Some("https://example.com/oa.pdf"));
    }

    #[test]
    fn rrf_merge_dedupes_by_doi_and_keeps_sources() {
        let a = AcademicPaper {
            id: "a".into(),
            title: "Same".into(),
            doi: Some("10.1/same".into()),
            sources: vec![Source::new("https://dblp.org/x", "dblp")],
            ..Default::default()
        };
        let b = AcademicPaper {
            id: "b".into(),
            title: "Same".into(),
            doi: Some("10.1/same".into()),
            citation_count: Some(10),
            sources: vec![Source::new("https://semanticscholar.org/x", "semantic")],
            ..Default::default()
        };
        let merged = rrf_merge(vec![("dblp".into(), vec![a]), ("semantic".into(), vec![b])]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].citation_count, Some(10));
        assert_eq!(merged[0].sources.len(), 2);
    }
}
