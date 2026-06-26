use std::collections::BTreeMap;

use async_trait::async_trait;
use grok_search_config::Config;
use grok_search_content::download_pdf_bytes;
use grok_search_parse::{
    parse_academic_identifier as parse_identifier, rrf_merge_papers as rrf_merge,
};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, AcademicServiceProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicCitationsOutput, AcademicGetOutput, AcademicPaper,
    AcademicReadOutput, AcademicSearchInput, AcademicSearchOutput, GrokSearchError, Result, Source,
};
use uuid::Uuid;

use crate::providers::{
    ArxivProvider, CrossrefProvider, DblpProvider, OpenAlexProvider, SciHubProvider,
    SemanticProvider, UnpaywallProvider,
};

pub(crate) const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";
const DEFAULT_SOURCES: &[&str] = &["dblp", "semantic", "arxiv"];
const ALL_SOURCES: &[&str] = &["dblp", "semantic", "arxiv", "openalex", "crossref"];

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
                openalex: OpenAlexProvider::new(
                    client.clone(),
                    config.academic_email.clone(),
                    config.openalex_api_key.clone(),
                ),
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
        let parsed = grok_search_content::parse_pdf_bytes(&bytes, &format, Some(limit))?;
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
            "openalex_api_key": self.config.openalex_key_status(),
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

#[async_trait]
impl AcademicServiceProvider for AcademicService {
    async fn search(&self, input: AcademicSearchInput) -> Result<AcademicSearchOutput> {
        AcademicService::search(self, input).await
    }

    async fn get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
    ) -> Result<AcademicGetOutput> {
        AcademicService::get(self, identifier, include_citations, include_open_access).await
    }

    async fn citations(&self, identifier: &str, limit: usize) -> Result<AcademicCitationsOutput> {
        AcademicService::citations(self, identifier, limit).await
    }

    async fn read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
    ) -> Result<AcademicReadOutput> {
        AcademicService::read(self, identifier, url, max_chars, output_format).await
    }

    fn diagnostics(&self) -> serde_json::Value {
        AcademicService::diagnostics(self)
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

pub(crate) fn source(
    url: impl Into<String>,
    provider: &'static str,
    title: Option<String>,
) -> Source {
    let mut source = Source::new(url, provider);
    source.title = title;
    source
}

pub(crate) fn as_u32(value: Option<u64>) -> Option<u32> {
    value.and_then(|v| u32::try_from(v).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{parse_dblp_search, parse_openalex_work, parse_semantic_paper};
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
