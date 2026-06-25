use async_trait::async_trait;
use grok_search_net::http::{get_json, get_text};
use grok_search_parse::{clean_html_title, extract_arxiv_id_from_path, openalex_abstract};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicPaper, AcademicSearchInput, GrokSearchError, Result,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::header::USER_AGENT;
use serde_json::Value;
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
}

impl SemanticProvider {
    pub(crate) fn new(client: reqwest::Client, api_key: Option<String>) -> Self {
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

#[derive(Clone)]
pub(crate) struct OpenAlexProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl OpenAlexProvider {
    pub(crate) fn new(client: reqwest::Client, email: Option<String>) -> Self {
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
