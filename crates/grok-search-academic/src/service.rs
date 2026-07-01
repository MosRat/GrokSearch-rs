use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::time::Instant;

use async_trait::async_trait;
use grok_search_cache::{PdfCachePut, PdfCacheStore, RedbPdfCache};
use grok_search_config::Config;
use grok_search_parse::{
    normalize_title, parse_academic_identifier as parse_identifier, rrf_merge_papers as rrf_merge,
};
use grok_search_pdf::{
    download_pdf_bytes_optimized, parse_pdf_bytes_detailed, OptimizedPdfDownloadOptions,
    ParsedPdfDetails,
};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, AcademicServiceProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicCitationsOutput, AcademicDownloadPdfOutput, AcademicGetOutput,
    AcademicLlmProgressiveOptions, AcademicMaterialLink, AcademicPaper, AcademicParseOptions,
    AcademicParsePdfOutput, AcademicPdfArtifactsInput, AcademicPdfArtifactsOutput,
    AcademicPdfCacheInfo, AcademicPdfCachePolicy, AcademicPdfDownloadInput,
    AcademicPdfDownloadOutput, AcademicPdfLocator, AcademicPdfReadInput, AcademicPdfReadOutput,
    AcademicPdfStructureInput, AcademicPdfStructureOutput, AcademicPdfStructureProfile,
    AcademicProgressiveGetInput, AcademicProgressiveGetOutput, AcademicProgressivePaper,
    AcademicReadOutput, AcademicSearchInput, AcademicSearchOutput, GrokSearchError, Result, Source,
};
use uuid::Uuid;

use crate::institutional::InstitutionalAccessManager;
use crate::llm_progressive;
use crate::providers::{
    without_openalex_reference_sources, ArxivProvider, CrossrefProvider, DblpProvider,
    OpenAlexProvider, SciHubProvider, SemanticProvider, UnpaywallProvider,
};

pub(crate) const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";
const DEFAULT_SOURCES: &[&str] = &["dblp", "semantic", "arxiv"];
const ALL_SOURCES: &[&str] = &["dblp", "semantic", "arxiv", "openalex", "crossref"];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcademicSearchMode {
    Balanced,
    Broad,
    Precise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcademicSortBy {
    Relevance,
    Citations,
    Date,
}

#[derive(Clone)]
pub struct AcademicService {
    client: reqwest::Client,
    config: Config,
    providers: ProviderSet,
    institutional: InstitutionalAccessManager,
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

struct ResolvedPaper {
    paper: AcademicPaper,
    chain: Vec<String>,
}

struct ReadDetails {
    identifier: Option<String>,
    url: Option<String>,
    pdf_url: String,
    parsed: ParsedPdfDetails,
    source: String,
    fulltext_status: String,
    resolver_chain: Vec<String>,
    metadata_materials: Vec<AcademicMaterialLink>,
    progressive_reading: Option<AcademicProgressivePaper>,
    pdf_cache: AcademicPdfCacheInfo,
}

struct PdfDownloadDetails {
    identifier: Option<String>,
    url: Option<String>,
    location: FullTextLocation,
    resolver_chain: Vec<String>,
}

struct DownloadedPdf {
    bytes: Vec<u8>,
    cache: AcademicPdfCacheInfo,
}

struct RemotePdfDownload {
    bytes: Vec<u8>,
    attempts: u32,
    backoff_ms: u64,
    plan: Option<String>,
    strategy: Option<String>,
    strategy_attempts: Vec<String>,
}

struct RemotePdfDownloadOnce {
    bytes: Vec<u8>,
    plan: Option<String>,
    strategy: Option<String>,
    strategy_attempts: Vec<String>,
}

struct ParsedPdfDownload {
    parsed: ParsedPdfDetails,
    cache: AcademicPdfCacheInfo,
}

impl AcademicService {
    pub fn new(client: reqwest::Client, config: Config) -> Self {
        Self {
            providers: ProviderSet {
                dblp: DblpProvider::new(client.clone()),
                semantic: SemanticProvider::new_with_limit(
                    client.clone(),
                    config.semantic_scholar_api_key.clone(),
                    config.max_response_bytes,
                ),
                arxiv: ArxivProvider::new(client.clone()),
                openalex: OpenAlexProvider::new_with_limit(
                    client.clone(),
                    config.academic_email.clone(),
                    config.openalex_api_key.clone(),
                    config.max_response_bytes,
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
            institutional: InstitutionalAccessManager::new(config.clone()),
            config,
        }
    }

    pub fn warm_institutional_access(&self) {
        self.institutional.warm();
    }

    pub async fn search(&self, input: AcademicSearchInput) -> Result<AcademicSearchOutput> {
        if input.query.trim().is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "academic_search.query is required".to_string(),
            ));
        }
        if input.max_results == Some(0) {
            return Ok(AcademicSearchOutput {
                session_id: short_session_id(),
                papers_count: 0,
                papers: Vec::new(),
                sources_used: Vec::new(),
                errors: BTreeMap::new(),
                truncated: false,
            });
        }
        let limit = input.max_results.unwrap_or(10).clamp(1, 50);
        let mode = search_mode(input.search_mode.as_deref())?;
        let sort_by = academic_sort_by(input.sort_by.as_deref())?;
        let selected = selected_sources(&input.sources, mode)?;
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
        if matches!(
            mode,
            AcademicSearchMode::Balanced | AcademicSearchMode::Precise
        ) {
            self.enrich_search_results(&mut papers, limit.saturating_mul(2), &mut errors)
                .await;
        }
        papers.retain(|paper| search_result_is_relevant(&input.query, paper));
        if matches!(sort_by, AcademicSortBy::Citations | AcademicSortBy::Date) {
            papers.retain(|paper| search_result_has_strong_overlap(&input.query, paper));
        }
        if mode == AcademicSearchMode::Precise {
            papers.retain(|paper| precise_search_result_is_relevant(&input.query, paper));
        }
        papers.retain(|paper| paper_matches_year_filter(paper, input.year_from, input.year_to));
        if input.open_access_only.unwrap_or(false) {
            papers.retain(|paper| paper.open_access.unwrap_or(false) || paper.pdf_url.is_some());
        }
        rank_academic_results(&input.query, sort_by, &mut papers);
        if input.include_abstract == Some(false) {
            for paper in &mut papers {
                paper.abstract_text = None;
            }
        }
        papers.truncate(limit);
        if input.extract_material_links.unwrap_or(false) {
            for paper in &mut papers {
                paper.materials = material_links_for_paper(paper);
            }
        }

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
        extract_material_links: bool,
    ) -> Result<AcademicGetOutput> {
        let mut resolved = self.resolve_canonical_paper(identifier).await?;
        resolved.paper = without_openalex_reference_sources(resolved.paper);
        if include_open_access {
            if let Ok(Some(location)) = self.resolve_fulltext_location(&resolved.paper).await {
                resolved.paper.pdf_url = resolved.paper.pdf_url.or(Some(location.url));
            }
        }
        let citations = if include_citations {
            self.citation_summary(&identifier_for_paper(&resolved.paper), 10)
                .await
                .map(clean_citation_summary)
                .ok()
        } else {
            None
        };
        if extract_material_links {
            resolved.paper.materials = material_links_for_paper(&resolved.paper);
        }
        Ok(AcademicGetOutput {
            paper: resolved.paper,
            citations,
            resolver_chain: resolved.chain,
        })
    }

    pub async fn citations(
        &self,
        identifier: &str,
        limit: usize,
    ) -> Result<AcademicCitationsOutput> {
        let resolved = self.resolve_canonical_paper(identifier).await?;
        let limit = limit.clamp(1, 50);
        let mut sources_used = Vec::new();
        let ids = citation_identifiers_for_paper(&resolved.paper);
        for provider in [
            &self.providers.semantic as &dyn AcademicProvider,
            &self.providers.openalex,
        ] {
            for id in &ids {
                match provider.citations(id, limit).await {
                    Ok(Some(summary))
                        if !summary.citations.is_empty() || !summary.references.is_empty() =>
                    {
                        sources_used.push(provider.name().to_string());
                        let summary = clean_citation_summary(summary);
                        return Ok(AcademicCitationsOutput {
                            identifier: identifier.to_string(),
                            citation_count: Some(summary.citations.len() as u32),
                            reference_count: Some(summary.references.len() as u32),
                            citations: summary.citations,
                            references: summary.references,
                            sources_used,
                        });
                    }
                    Ok(Some(_)) | Ok(None) => {}
                    Err(_) => {}
                }
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
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicReadOutput> {
        let include_parse_details = parse_options.is_some();
        let include_materials = parse_options
            .as_ref()
            .and_then(|options| options.extract_material_links)
            .unwrap_or(false);
        let details = self
            .read_pdf_details(
                identifier,
                url,
                max_chars,
                output_format,
                parse_options,
                AcademicPdfCachePolicy::Auto,
            )
            .await?;
        let materials = if include_materials {
            merge_materials(
                details.metadata_materials,
                material_links_from_text(&details.parsed.content, "pdf_content"),
            )
        } else {
            Vec::new()
        };
        Ok(AcademicReadOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            content: details.parsed.content,
            original_length: details.parsed.original_length,
            truncated: details.parsed.truncated,
            raw_content: details.parsed.raw_content,
            raw_original_length: details.parsed.raw_original_length,
            raw_truncated: details.parsed.raw_truncated,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            artifacts: details.parsed.artifacts,
            parse_capabilities: include_parse_details.then_some(details.parsed.capabilities),
            processing: include_parse_details.then_some(details.parsed.processing),
            materials,
            progressive_reading: details.progressive_reading,
            pdf_cache: Some(details.pdf_cache),
        })
    }

    pub async fn parse_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicParsePdfOutput> {
        let include_materials = parse_options
            .as_ref()
            .and_then(|options| options.extract_material_links)
            .unwrap_or(true);
        let details = self
            .read_pdf_details(
                identifier,
                url,
                max_chars,
                output_format,
                parse_options,
                AcademicPdfCachePolicy::Auto,
            )
            .await?;
        let materials = if include_materials {
            merge_materials(
                details.metadata_materials,
                material_links_from_text(&details.parsed.content, "pdf_content"),
            )
        } else {
            Vec::new()
        };
        Ok(AcademicParsePdfOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            content: details.parsed.content,
            original_length: details.parsed.original_length,
            truncated: details.parsed.truncated,
            raw_content: details.parsed.raw_content,
            raw_original_length: details.parsed.raw_original_length,
            raw_truncated: details.parsed.raw_truncated,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            artifacts: details.parsed.artifacts,
            parse_capabilities: details.parsed.capabilities,
            processing: details.parsed.processing,
            materials,
            progressive_reading: details.progressive_reading,
            pdf_cache: Some(details.pdf_cache),
        })
    }

    pub async fn download_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        output_path: String,
        overwrite: bool,
    ) -> Result<AcademicDownloadPdfOutput> {
        self.pdf_download(AcademicPdfDownloadInput {
            locator: AcademicPdfLocator {
                identifier,
                url,
                pdf_url: None,
            },
            output_path,
            overwrite: Some(overwrite),
            cache_policy: None,
        })
        .await
    }

    pub async fn pdf_read(&self, input: AcademicPdfReadInput) -> Result<AcademicPdfReadOutput> {
        ensure_valid_locator(&input.locator, "academic_pdf_read")?;
        let parse_options = AcademicParseOptions {
            text_processing_mode: input.text_mode,
            include_raw_content: input.include_raw_content,
            extract_material_links: input.extract_material_links,
            ..Default::default()
        };
        let include_processing = input.include_processing.unwrap_or(false);
        let include_materials = input.extract_material_links.unwrap_or(false);
        let details = self
            .read_pdf_details_from_locator(
                input.locator,
                input.max_chars,
                Some("markdown".to_string()),
                Some(parse_options),
                input.cache_policy.unwrap_or_default(),
                "academic_pdf_read",
            )
            .await?;
        let materials = if include_materials {
            merge_materials(
                details.metadata_materials,
                material_links_from_text(&details.parsed.content, "pdf_content"),
            )
        } else {
            Vec::new()
        };
        Ok(AcademicReadOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            content: details.parsed.content,
            original_length: details.parsed.original_length,
            truncated: details.parsed.truncated,
            raw_content: details.parsed.raw_content,
            raw_original_length: details.parsed.raw_original_length,
            raw_truncated: details.parsed.raw_truncated,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            artifacts: details.parsed.artifacts,
            parse_capabilities: include_processing.then_some(details.parsed.capabilities),
            processing: include_processing.then_some(details.parsed.processing),
            materials,
            progressive_reading: None,
            pdf_cache: Some(details.pdf_cache),
        })
    }

    pub async fn pdf_structure(
        &self,
        input: AcademicPdfStructureInput,
    ) -> Result<AcademicPdfStructureOutput> {
        ensure_valid_locator(&input.locator, "academic_pdf_structure")?;
        let view = input.view.clone().unwrap_or_else(|| "summary".to_string());
        if !matches!(view.as_str(), "summary" | "full" | "section") {
            return Err(GrokSearchError::InvalidParams(
                "academic_pdf_structure.view must be one of summary, full, section".to_string(),
            ));
        }
        if view == "section" && input.section_id.as_deref().unwrap_or("").trim().is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "academic_pdf_structure.section_id is required when view=section".to_string(),
            ));
        }
        let llm = llm_options_for_structure(&input, &self.config);
        let parse_options = AcademicParseOptions {
            llm_progressive: Some(llm),
            ..Default::default()
        };
        let details = self
            .read_pdf_details_from_locator(
                input.locator,
                input.max_chars,
                Some("markdown".to_string()),
                Some(parse_options),
                input.cache_policy.unwrap_or_default(),
                "academic_pdf_structure",
            )
            .await?;
        let mut paper = details.progressive_reading.ok_or_else(|| {
            GrokSearchError::Provider(
                "academic_pdf_structure did not produce progressive_reading".to_string(),
            )
        })?;
        if input.include_section_text != Some(true) {
            for section in &mut paper.sections {
                section.clean_text = None;
            }
        }
        let section = if view == "section" {
            let section_id = input.section_id.as_deref().unwrap_or_default();
            Some(
                paper
                    .sections
                    .iter()
                    .find(|section| section.section_id == section_id)
                    .cloned()
                    .ok_or_else(|| {
                        GrokSearchError::NotFound(format!(
                            "progressive section {section_id} not found"
                        ))
                    })?,
            )
        } else {
            None
        };
        Ok(AcademicPdfStructureOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            view,
            progressive_reading: paper,
            section,
            processing: details.parsed.processing,
            artifacts: details.parsed.artifacts,
            pdf_cache: Some(details.pdf_cache),
        })
    }

    pub async fn pdf_artifacts(
        &self,
        input: AcademicPdfArtifactsInput,
    ) -> Result<AcademicPdfArtifactsOutput> {
        ensure_valid_locator(&input.locator, "academic_pdf_artifacts")?;
        let parse_options = AcademicParseOptions {
            images_dir: input.images_dir,
            tables_dir: input.tables_dir,
            extract_images: input.extract_images,
            extract_tables: input.extract_tables,
            text_processing_mode: input.text_mode,
            ..Default::default()
        };
        let details = self
            .read_pdf_details_from_locator(
                input.locator,
                input.max_chars,
                Some("markdown".to_string()),
                Some(parse_options),
                input.cache_policy.unwrap_or_default(),
                "academic_pdf_artifacts",
            )
            .await?;
        Ok(AcademicPdfArtifactsOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            artifacts: details.parsed.artifacts,
            parse_capabilities: details.parsed.capabilities,
            processing: details.parsed.processing,
            pdf_cache: Some(details.pdf_cache),
        })
    }

    pub async fn pdf_download(
        &self,
        input: AcademicPdfDownloadInput,
    ) -> Result<AcademicPdfDownloadOutput> {
        ensure_valid_locator(&input.locator, "academic_pdf_download")?;
        let identifier = input.locator.identifier;
        let url = input.locator.url.or(input.locator.pdf_url);
        let output_path = input.output_path;
        let overwrite = input.overwrite.unwrap_or(false);
        let details = self.resolve_pdf_download_details(identifier, url).await?;
        let path = std::path::PathBuf::from(output_path);
        if path.exists() && !overwrite {
            return Err(GrokSearchError::InvalidParams(format!(
                "PDF output path already exists: {}",
                path.display()
            )));
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                GrokSearchError::Io(format!(
                    "create PDF output directory {}: {err}",
                    parent.display()
                ))
            })?;
        }
        let bytes = self
            .download_pdf_for_location(&details.location, input.cache_policy.unwrap_or_default())
            .await?;
        std::fs::write(&path, &bytes.bytes).map_err(|err| {
            GrokSearchError::Io(format!("write PDF output {}: {err}", path.display()))
        })?;
        Ok(AcademicDownloadPdfOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.location.url,
            source: details.location.source,
            fulltext_status: details.location.status,
            resolver_chain: details.resolver_chain,
            path: path.display().to_string(),
            bytes: bytes.bytes.len() as u64,
            pdf_cache: Some(bytes.cache),
        })
    }

    pub async fn progressive_get(
        &self,
        input: AcademicProgressiveGetInput,
    ) -> Result<AcademicProgressiveGetOutput> {
        llm_progressive::get_cached(input, &self.config).await
    }

    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.config.academic_enabled,
            "semantic_scholar_api_key": self.config.semantic_scholar_key_status(),
            "openalex_api_key": self.config.openalex_key_status(),
            "unpaywall_email": self.config.academic_email_status(),
            "scihub_enabled": self.config.academic_scihub_enabled,
            "scihub_base_url": self.config.redacted_scihub_base_url(),
            "institutional": serde_json::json!({
                "enabled": self.config.academic_institutional_enabled,
                "status": "pending",
                "detail": "institutional access probe has not completed",
                "ieee": { "available": false, "route": serde_json::Value::Null, "source": serde_json::Value::Null, "proxy_url": serde_json::Value::Null },
                "acm": { "available": false, "route": serde_json::Value::Null, "source": serde_json::Value::Null, "proxy_url": serde_json::Value::Null },
            }),
            "pdf_parser": "pdf_oxide",
            "providers": ["dblp", "semantic", "arxiv", "openalex", "crossref", "unpaywall"],
        })
    }

    pub async fn diagnostics_live(&self) -> serde_json::Value {
        let mut value = self.diagnostics();
        value["institutional"] = self.institutional.diagnostics(true).await;
        value
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

    async fn resolve_canonical_paper(&self, identifier: &str) -> Result<ResolvedPaper> {
        let id = parse_identifier(identifier);
        let mut chain = Vec::new();
        let mut candidates = Vec::new();
        for provider in self.get_providers() {
            chain.push(provider.name().to_string());
            match provider.get(&id).await {
                Ok(Some(found)) if resolved_paper_matches_identifier(&id, &found) => {
                    candidates.push(found);
                }
                Ok(_) | Err(_) => {}
            }
        }
        if !candidates.is_empty() {
            let paper = if matches!(id, Identifier::Query(_)) {
                select_best_title_match(identifier, candidates).ok_or_else(|| {
                    GrokSearchError::NotFound(format!(
                        "academic identifier not found: {identifier}"
                    ))
                })?
            } else {
                merge_canonical_candidates(candidates)
            };
            return Ok(ResolvedPaper { paper, chain });
        }
        if matches!(id, Identifier::Query(_)) {
            let fallback = self.title_query_fallback(identifier).await?;
            chain.extend(fallback.resolver_chain);
            return Ok(ResolvedPaper {
                paper: fallback.paper,
                chain,
            });
        }
        Err(GrokSearchError::NotFound(format!(
            "academic identifier not found: {identifier}"
        )))
    }

    async fn resolve_fulltext_location(
        &self,
        paper: &AcademicPaper,
    ) -> Result<Option<FullTextLocation>> {
        Ok(self
            .resolve_fulltext_locations(paper)
            .await?
            .into_iter()
            .next())
    }

    async fn resolve_fulltext_locations(
        &self,
        paper: &AcademicPaper,
    ) -> Result<Vec<FullTextLocation>> {
        let mut locations = Vec::new();
        if let Some(url) = &paper.pdf_url {
            locations.push(FullTextLocation {
                url: url.clone(),
                source: "paper".to_string(),
                status: "paper_pdf_url".to_string(),
            });
        }
        if let Some(arxiv_id) = &paper.arxiv_id {
            locations.push(FullTextLocation {
                url: format!("https://arxiv.org/pdf/{arxiv_id}"),
                source: "arxiv".to_string(),
                status: "arxiv_pdf".to_string(),
            });
        }
        for provider in [
            &self.providers.arxiv as &dyn AcademicProvider,
            &self.providers.semantic,
            &self.providers.openalex,
            &self.providers.unpaywall,
        ] {
            if let Some(location) = provider.resolve_fulltext(paper).await? {
                locations.push(location);
            }
        }
        locations.extend(self.institutional.resolve_locations(paper).await);
        if let Some(location) = self.providers.scihub.resolve_fulltext(paper).await? {
            locations.push(location);
        }
        Ok(prefer_institutional_locations(locations))
    }

    async fn download_and_parse_pdf(
        &self,
        location: &FullTextLocation,
        format: String,
        limit: usize,
        options: Option<&AcademicParseOptions>,
        cache_policy: AcademicPdfCachePolicy,
    ) -> Result<ParsedPdfDownload> {
        let download = self
            .download_pdf_for_location(location, cache_policy)
            .await?;
        let parsed = parse_pdf_bytes_with_timeout(
            download.bytes,
            format,
            limit,
            options,
            self.config.timeout,
            &location.url,
        )
        .await?;
        Ok(ParsedPdfDownload {
            parsed,
            cache: download.cache,
        })
    }

    async fn download_pdf_for_location(
        &self,
        location: &FullTextLocation,
        cache_policy: AcademicPdfCachePolicy,
    ) -> Result<DownloadedPdf> {
        let cache_key = pdf_cache_key(location, self.config.academic_max_pdf_bytes);
        let mut cache_info = AcademicPdfCacheInfo {
            key: cache_key.clone(),
            ..Default::default()
        };
        let cache_allowed = self.config.academic_pdf_cache_enabled
            && !matches!(cache_policy, AcademicPdfCachePolicy::Bypass);
        let refresh = matches!(cache_policy, AcademicPdfCachePolicy::Refresh);

        if cache_allowed && !refresh {
            match RedbPdfCache::open(&self.config.academic_pdf_cache_path) {
                Ok(cache) => match cache.get(&cache_key) {
                    Ok(Some(entry)) => {
                        cache_info.hit = true;
                        cache_info.bytes = entry.bytes.len() as u64;
                        cache_info.attempts = 0;
                        cache_info.download_elapsed_ms = 0;
                        if grok_search_pdf::validate_pdf_bytes(
                            &entry.bytes,
                            self.config.academic_max_pdf_bytes,
                        )
                        .is_ok()
                        {
                            return Ok(DownloadedPdf {
                                bytes: entry.bytes,
                                cache: cache_info,
                            });
                        }
                        cache_info.hit = false;
                        cache_info
                            .warnings
                            .push("cached PDF bytes failed validation; refetching".to_string());
                        if let Err(err) = cache.remove(&cache_key) {
                            cache_info
                                .warnings
                                .push(format!("failed to remove invalid PDF cache entry: {err}"));
                        }
                    }
                    Ok(None) => {}
                    Err(err) => cache_info
                        .warnings
                        .push(format!("PDF cache read failed: {err}")),
                },
                Err(err) => cache_info
                    .warnings
                    .push(format!("PDF cache open failed: {err}")),
            }
        }

        let started = Instant::now();
        let remote = self.download_pdf_remote_with_backoff(location).await;
        cache_info.download_elapsed_ms = started.elapsed().as_millis() as u64;
        let remote = remote?;
        cache_info.attempts = remote.attempts;
        cache_info.backoff_ms = remote.backoff_ms;
        cache_info.bytes = remote.bytes.len() as u64;
        if let Some(plan) = remote.plan {
            cache_info.warnings.push(format!("download_plan={plan}"));
        }
        if let Some(strategy) = remote.strategy {
            cache_info
                .warnings
                .push(format!("download_strategy={strategy}"));
        }
        cache_info.warnings.extend(remote.strategy_attempts);

        if cache_allowed {
            match RedbPdfCache::open(&self.config.academic_pdf_cache_path) {
                Ok(cache) => {
                    let put = PdfCachePut {
                        cache_key: cache_key.clone(),
                        bytes: remote.bytes.clone(),
                        ttl_seconds: Some(self.config.academic_pdf_cache_ttl_seconds),
                        pdf_sha256: sha256_hex(&remote.bytes),
                        source: location.source.clone(),
                        host: pdf_url_host(&location.url),
                    };
                    match cache.put(
                        put,
                        self.config.academic_pdf_cache_max_entries,
                        self.config.academic_pdf_cache_max_bytes as u64,
                    ) {
                        Ok(_) => cache_info.stored = true,
                        Err(err) => cache_info
                            .warnings
                            .push(format!("PDF cache write failed: {err}")),
                    }
                }
                Err(err) => cache_info
                    .warnings
                    .push(format!("PDF cache open failed: {err}")),
            }
        }

        Ok(DownloadedPdf {
            bytes: remote.bytes,
            cache: cache_info,
        })
    }

    async fn download_pdf_remote_with_backoff(
        &self,
        location: &FullTextLocation,
    ) -> Result<RemotePdfDownload> {
        let mut backoff_ms = 0;
        let mut last_err = None;
        for attempt in 1..=3 {
            match self.download_pdf_remote_once(location).await {
                Ok(once) => {
                    return Ok(RemotePdfDownload {
                        bytes: once.bytes,
                        attempts: attempt,
                        backoff_ms,
                        plan: once.plan,
                        strategy: once.strategy,
                        strategy_attempts: once.strategy_attempts,
                    })
                }
                Err(err) if attempt < 3 && is_retryable_pdf_download_error(&err) => {
                    let delay = pdf_download_retry_delay_ms(attempt);
                    backoff_ms += delay;
                    last_err = Some(err);
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            GrokSearchError::Upstream(format!("academic PDF download failed for {}", location.url))
        }))
    }

    async fn download_pdf_remote_once(
        &self,
        location: &FullTextLocation,
    ) -> Result<RemotePdfDownloadOnce> {
        if matches!(
            location.source.as_str(),
            "ieee_institutional" | "acm_institutional"
        ) {
            let bytes = tokio::time::timeout(
                self.config.timeout,
                self.institutional
                    .download_pdf(location, self.config.academic_max_pdf_bytes),
            )
            .await
            .map_err(|_| {
                GrokSearchError::Timeout(format!(
                    "academic_read PDF download timed out for {}",
                    location.url
                ))
            })??;
            Ok(RemotePdfDownloadOnce {
                bytes,
                plan: None,
                strategy: Some("institutional_full".to_string()),
                strategy_attempts: Vec::new(),
            })
        } else {
            let mut options = OptimizedPdfDownloadOptions::new(
                self.config.timeout,
                self.config.academic_max_pdf_bytes,
                self.config.max_response_bytes,
            );
            options.enable_direct_fallback = !self.config.proxy.trim().eq_ignore_ascii_case("off");
            let outcome =
                download_pdf_bytes_optimized(&self.client, &location.url, options).await?;
            let strategy_attempts = outcome
                .attempts
                .iter()
                .map(|attempt| {
                    format!(
                        "download_attempt strategy={} status={} elapsed_ms={} bytes={}{}",
                        attempt.strategy,
                        attempt.status,
                        attempt.elapsed_ms,
                        attempt.bytes,
                        attempt
                            .error
                            .as_ref()
                            .map(|err| format!(" error={err}"))
                            .unwrap_or_default()
                    )
                })
                .collect();
            Ok(RemotePdfDownloadOnce {
                bytes: outcome.bytes,
                plan: Some(outcome.plan),
                strategy: Some(outcome.strategy),
                strategy_attempts,
            })
        }
    }

    async fn resolve_pdf_download_details(
        &self,
        identifier: Option<String>,
        url: Option<String>,
    ) -> Result<PdfDownloadDetails> {
        let mut chain = Vec::new();
        let locations = if let Some(url) = url.clone() {
            if let Some(location) = self.institutional.resolve_url_location(&url).await {
                vec![location]
            } else {
                vec![FullTextLocation {
                    url,
                    source: "direct_url".to_string(),
                    status: "direct_url".to_string(),
                }]
            }
        } else {
            let identifier_ref = identifier.as_deref().ok_or_else(|| {
                GrokSearchError::InvalidParams(
                    "academic_download_pdf requires identifier or url".into(),
                )
            })?;
            let get = self.get(identifier_ref, false, true, true).await?;
            chain.extend(get.resolver_chain);
            let locations = self.resolve_fulltext_locations(&get.paper).await?;
            if locations.is_empty() {
                return Err(GrokSearchError::NotFound(
                    "no full-text PDF URL found".into(),
                ));
            }
            locations
        };
        let mut locations = prefer_institutional_locations(locations);
        let location = locations
            .drain(..)
            .next()
            .ok_or_else(|| GrokSearchError::NotFound("no full-text PDF URL found".into()))?;
        chain.push(location.source.clone());
        Ok(PdfDownloadDetails {
            identifier,
            url,
            location,
            resolver_chain: chain,
        })
    }

    async fn read_pdf_details_from_locator(
        &self,
        locator: AcademicPdfLocator,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
        cache_policy: AcademicPdfCachePolicy,
        tool_name: &str,
    ) -> Result<ReadDetails> {
        ensure_valid_locator(&locator, tool_name)?;
        let AcademicPdfLocator {
            identifier,
            url,
            pdf_url,
        } = locator;
        self.read_pdf_details(
            identifier,
            url.or(pdf_url),
            max_chars,
            output_format,
            parse_options,
            cache_policy,
        )
        .await
    }

    async fn read_pdf_details(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
        cache_policy: AcademicPdfCachePolicy,
    ) -> Result<ReadDetails> {
        let format = output_format.unwrap_or_else(|| "markdown".to_string());
        if format != "markdown" && format != "text" {
            return Err(GrokSearchError::InvalidParams(
                "output_format must be \"markdown\" or \"text\"".to_string(),
            ));
        }
        let mut chain = Vec::new();
        let mut metadata_materials = Vec::new();
        let locations = if let Some(url) = url.clone() {
            metadata_materials.extend(material_links_from_url(&url, "input_url"));
            if let Some(location) = self.institutional.resolve_url_location(&url).await {
                vec![location]
            } else {
                vec![FullTextLocation {
                    url,
                    source: "direct_url".to_string(),
                    status: "direct_url".to_string(),
                }]
            }
        } else {
            let identifier_ref = identifier.as_deref().ok_or_else(|| {
                GrokSearchError::InvalidParams("academic_read requires identifier or url".into())
            })?;
            let get = self.get(identifier_ref, false, true, true).await?;
            metadata_materials.extend(material_links_for_paper(&get.paper));
            chain.extend(get.resolver_chain);
            let locations = self.resolve_fulltext_locations(&get.paper).await?;
            if locations.is_empty() {
                return Err(GrokSearchError::NotFound(
                    "no full-text PDF URL found".into(),
                ));
            }
            locations
        };
        let locations = prefer_institutional_locations(locations);
        let limit = max_chars
            .or(self.config.academic_pdf_max_chars)
            .or(self.config.fetch_max_chars)
            .unwrap_or(200_000);
        let mut failures = Vec::new();
        for location in locations {
            match self
                .download_and_parse_pdf(
                    &location,
                    format.clone(),
                    limit,
                    parse_options.as_ref(),
                    cache_policy,
                )
                .await
            {
                Ok(mut parsed_download) => {
                    let progressive = self
                        .maybe_run_llm_progressive(
                            &mut parsed_download.parsed,
                            parse_options.as_ref(),
                        )
                        .await;
                    chain.push(location.source.clone());
                    return Ok(ReadDetails {
                        identifier,
                        url,
                        pdf_url: location.url,
                        parsed: parsed_download.parsed,
                        source: location.source,
                        fulltext_status: location.status,
                        resolver_chain: chain,
                        metadata_materials,
                        progressive_reading: progressive,
                        pdf_cache: parsed_download.cache,
                    });
                }
                Err(err) => {
                    failures.push((err.kind().to_string(), format!("{}: {err}", location.url)))
                }
            }
        }
        let message = failures
            .iter()
            .map(|(_, message)| message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        let err = format!("academic_read PDF fetch failed for all candidates: {message}");
        if failures.iter().any(|(kind, message)| {
            kind == "timeout"
                || message.to_ascii_lowercase().contains("timeout")
                || message.to_ascii_lowercase().contains("timed out")
        }) {
            return Err(GrokSearchError::Timeout(err));
        }
        if failures.iter().any(|(kind, _)| kind == "upstream") {
            return Err(GrokSearchError::Upstream(err));
        }
        Err(GrokSearchError::Provider(err))
    }

    async fn maybe_run_llm_progressive(
        &self,
        parsed: &mut ParsedPdfDetails,
        parse_options: Option<&AcademicParseOptions>,
    ) -> Option<AcademicProgressivePaper> {
        let Some(options) = parse_options.and_then(|options| options.llm_progressive.as_ref())
        else {
            return None;
        };
        if !llm_progressive::enabled(Some(options)) {
            return None;
        }
        let outcome = llm_progressive::run(parsed, options, &self.config, &self.client).await;
        if let Some(artifact) = outcome.artifact {
            parsed.artifacts.push(artifact);
        }
        parsed.processing.passes.push(outcome.pass.clone());
        parsed.processing.warnings.extend(outcome.pass.warnings);
        outcome.value
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

    async fn title_query_fallback(&self, identifier: &str) -> Result<AcademicGetOutput> {
        let search = self
            .search(AcademicSearchInput {
                query: identifier.to_string(),
                sources: ALL_SOURCES
                    .iter()
                    .map(|source| source.to_string())
                    .collect(),
                search_mode: Some("precise".to_string()),
                sort_by: Some("relevance".to_string()),
                max_results: Some(10),
                ..Default::default()
            })
            .await?;
        let paper = select_best_title_match(identifier, search.papers).ok_or_else(|| {
            GrokSearchError::NotFound(format!("academic identifier not found: {identifier}"))
        })?;
        Ok(AcademicGetOutput {
            paper,
            citations: None,
            resolver_chain: vec!["search_fallback".to_string()],
        })
    }

    async fn enrich_search_results(
        &self,
        papers: &mut [AcademicPaper],
        max_papers: usize,
        errors: &mut BTreeMap<String, String>,
    ) {
        for paper in papers.iter_mut().take(max_papers) {
            let id = identifier_for_paper(paper);
            for provider in [
                &self.providers.semantic as &dyn AcademicProvider,
                &self.providers.openalex,
                &self.providers.crossref,
            ] {
                match provider.get(&id).await {
                    Ok(Some(enriched)) => {
                        let enriched = if provider.name() == "openalex" {
                            without_openalex_reference_sources(enriched)
                        } else {
                            enriched
                        };
                        paper.merge_from(enriched);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        errors
                            .entry(format!("{}_enrichment", provider.name()))
                            .or_insert_with(|| err.to_string());
                    }
                }
            }
        }
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
        extract_material_links: bool,
    ) -> Result<AcademicGetOutput> {
        AcademicService::get(
            self,
            identifier,
            include_citations,
            include_open_access,
            extract_material_links,
        )
        .await
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
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicReadOutput> {
        AcademicService::read(
            self,
            identifier,
            url,
            max_chars,
            output_format,
            parse_options,
        )
        .await
    }

    async fn parse_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicParsePdfOutput> {
        AcademicService::parse_pdf(
            self,
            identifier,
            url,
            max_chars,
            output_format,
            parse_options,
        )
        .await
    }

    async fn download_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        output_path: String,
        overwrite: bool,
    ) -> Result<AcademicDownloadPdfOutput> {
        AcademicService::download_pdf(self, identifier, url, output_path, overwrite).await
    }

    async fn pdf_read(&self, input: AcademicPdfReadInput) -> Result<AcademicPdfReadOutput> {
        AcademicService::pdf_read(self, input).await
    }

    async fn pdf_structure(
        &self,
        input: AcademicPdfStructureInput,
    ) -> Result<AcademicPdfStructureOutput> {
        AcademicService::pdf_structure(self, input).await
    }

    async fn pdf_artifacts(
        &self,
        input: AcademicPdfArtifactsInput,
    ) -> Result<AcademicPdfArtifactsOutput> {
        AcademicService::pdf_artifacts(self, input).await
    }

    async fn pdf_download(
        &self,
        input: AcademicPdfDownloadInput,
    ) -> Result<AcademicPdfDownloadOutput> {
        AcademicService::pdf_download(self, input).await
    }

    async fn progressive_get(
        &self,
        input: AcademicProgressiveGetInput,
    ) -> Result<AcademicProgressiveGetOutput> {
        AcademicService::progressive_get(self, input).await
    }

    fn diagnostics(&self) -> serde_json::Value {
        AcademicService::diagnostics(self)
    }

    async fn diagnostics_live(&self) -> serde_json::Value {
        AcademicService::diagnostics_live(self).await
    }

    fn warm_institutional_access(&self) {
        AcademicService::warm_institutional_access(self)
    }
}

fn ensure_valid_locator(locator: &AcademicPdfLocator, tool_name: &str) -> Result<()> {
    if locator.is_valid_exactly_one() {
        return Ok(());
    }
    Err(GrokSearchError::InvalidParams(format!(
        "{tool_name} requires exactly one of identifier, url, or pdf_url"
    )))
}

fn pdf_cache_key(location: &FullTextLocation, max_bytes: usize) -> String {
    let payload = format!(
        "academic_pdf:v1\nsource={}\nurl={}\nmax_bytes={max_bytes}",
        location.source, location.url
    );
    format!("academic_pdf:v1:{}", sha256_hex(payload.as_bytes()))
}

fn pdf_url_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn pdf_download_retry_delay_ms(attempt: u32) -> u64 {
    match attempt {
        0 | 1 => 600,
        2 => 1_200,
        _ => 2_400,
    }
}

fn is_retryable_pdf_download_error(err: &GrokSearchError) -> bool {
    match err {
        GrokSearchError::Timeout(_) => true,
        GrokSearchError::Upstream(message) | GrokSearchError::Provider(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("http 429")
                || lower.contains("too many requests")
                || lower.contains("rate limit")
                || lower.contains("http 500")
                || lower.contains("http 502")
                || lower.contains("http 503")
                || lower.contains("http 504")
                || lower.contains("timed out")
                || lower.contains("timeout")
                || lower.contains("connect")
                || lower.contains("connection")
                || lower.contains("body read failed")
                || lower.contains("request failed")
        }
        _ => false,
    }
}

fn llm_options_for_structure(
    input: &AcademicPdfStructureInput,
    config: &Config,
) -> AcademicLlmProgressiveOptions {
    let profile = input.profile.unwrap_or_else(|| {
        profile_from_config(&config.progressive_default_profile)
            .unwrap_or(AcademicPdfStructureProfile::Balanced)
    });
    let mut options = match profile {
        AcademicPdfStructureProfile::Fast => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(4_500),
            overlap_chars: Some(300),
            concurrency: Some(3),
            max_output_tokens: Some(1_000),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
        AcademicPdfStructureProfile::Balanced => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(6_500),
            overlap_chars: Some(500),
            concurrency: Some(2),
            max_output_tokens: Some(1_600),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
        AcademicPdfStructureProfile::Strict => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(5_500),
            overlap_chars: Some(700),
            concurrency: Some(1),
            max_output_tokens: Some(1_800),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
    };
    options.enabled = Some(true);
    options.model = input
        .model
        .clone()
        .filter(|model| !model.trim().is_empty())
        .or_else(|| Some(config.progressive_default_model.clone()));
    options.save_json_path = input.save_json_path.clone();
    options.include_section_text = input.include_section_text;
    match input.cache_policy.unwrap_or_default() {
        AcademicPdfCachePolicy::Auto => {
            options.cache_enabled = Some(true);
            options.cache_refresh = Some(false);
        }
        AcademicPdfCachePolicy::Refresh => {
            options.cache_enabled = Some(true);
            options.cache_refresh = Some(true);
        }
        AcademicPdfCachePolicy::Bypass => {
            options.cache_enabled = Some(false);
            options.cache_refresh = Some(false);
        }
    }
    options
}

fn profile_from_config(raw: &str) -> Option<AcademicPdfStructureProfile> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "fast" => Some(AcademicPdfStructureProfile::Fast),
        "" | "balanced" => Some(AcademicPdfStructureProfile::Balanced),
        "strict" => Some(AcademicPdfStructureProfile::Strict),
        _ => None,
    }
}

fn search_mode(raw: Option<&str>) -> Result<AcademicSearchMode> {
    match raw
        .unwrap_or("balanced")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "balanced" => Ok(AcademicSearchMode::Balanced),
        "broad" => Ok(AcademicSearchMode::Broad),
        "precise" => Ok(AcademicSearchMode::Precise),
        other => Err(GrokSearchError::InvalidParams(format!(
            "search_mode must be \"balanced\", \"broad\", or \"precise\", got \"{other}\""
        ))),
    }
}

fn academic_sort_by(raw: Option<&str>) -> Result<AcademicSortBy> {
    match raw
        .unwrap_or("relevance")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "relevance" => Ok(AcademicSortBy::Relevance),
        "citations" => Ok(AcademicSortBy::Citations),
        "date" => Ok(AcademicSortBy::Date),
        other => Err(GrokSearchError::InvalidParams(format!(
            "sort_by must be \"relevance\", \"citations\", or \"date\", got \"{other}\""
        ))),
    }
}

fn selected_sources(raw: &[String], mode: AcademicSearchMode) -> Result<Vec<String>> {
    let requested: Vec<String> = if raw.is_empty() {
        let defaults = match mode {
            AcademicSearchMode::Balanced | AcademicSearchMode::Precise => DEFAULT_SOURCES,
            AcademicSearchMode::Broad => ALL_SOURCES,
        };
        defaults.iter().map(|s| s.to_string()).collect()
    } else {
        raw.iter()
            .flat_map(|s| s.split(','))
            .map(canonical_academic_source)
            .filter(|s| !s.is_empty())
            .collect()
    };
    let unknown = requested
        .iter()
        .find(|source| !ALL_SOURCES.contains(&source.as_str()));
    if let Some(source) = unknown {
        return Err(GrokSearchError::InvalidParams(format!(
            "unknown academic source: {source}; expected one of {}",
            ALL_SOURCES.join(", ")
        )));
    }
    Ok(requested)
}

fn canonical_academic_source(source: &str) -> String {
    match source.trim().to_ascii_lowercase().as_str() {
        "semantic_scholar" | "semanticscholar" | "s2" => "semantic".to_string(),
        other => other.to_string(),
    }
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

fn citation_identifiers_for_paper(paper: &AcademicPaper) -> Vec<Identifier> {
    let mut ids = Vec::new();
    if let Some(id) = &paper.semantic_scholar_id {
        ids.push(Identifier::Semantic(id.clone()));
    }
    if let Some(id) = &paper.openalex_id {
        ids.push(Identifier::OpenAlex(id.clone()));
    }
    if let Some(doi) = &paper.doi {
        ids.push(Identifier::Doi(doi.clone()));
    }
    if let Some(id) = &paper.arxiv_id {
        ids.push(Identifier::Arxiv(id.clone()));
    }
    if ids.is_empty() {
        ids.push(Identifier::Query(paper.title.clone()));
    }
    ids
}

fn resolved_paper_matches_identifier(id: &Identifier, paper: &AcademicPaper) -> bool {
    match id {
        Identifier::Query(query) => normalize_title(&paper.title) == normalize_title(query),
        _ => true,
    }
}

fn select_best_title_match(
    query: &str,
    papers: impl IntoIterator<Item = AcademicPaper>,
) -> Option<AcademicPaper> {
    let expected = normalize_title(query);
    let mut matches: Vec<AcademicPaper> = papers
        .into_iter()
        .filter(|paper| normalize_title(&paper.title) == expected)
        .collect();
    matches.sort_by_key(|paper| Reverse(canonical_title_score(query, paper)));
    matches.into_iter().next()
}

fn merge_canonical_candidates(mut candidates: Vec<AcademicPaper>) -> AcademicPaper {
    candidates.sort_by_key(|paper| Reverse(canonical_identifier_score(paper)));
    let mut merged = candidates.remove(0);
    for candidate in candidates {
        merged.merge_from(candidate);
    }
    merged
}

fn canonical_title_score(query: &str, paper: &AcademicPaper) -> u32 {
    let exact_title = (normalize_title(&paper.title) == normalize_title(query)) as u32;
    exact_title * 10_000
        + canonical_identifier_score(paper)
        + canonical_source_score(paper)
        + author_signal_score(paper)
        + venue_signal_score(paper)
        + stable_year_score(paper)
        + citation_signal_score(paper)
        + suspicious_doi_penalty(paper)
}

fn canonical_identifier_score(paper: &AcademicPaper) -> u32 {
    paper.semantic_scholar_id.is_some() as u32 * 2_000
        + paper.arxiv_id.is_some() as u32 * 1_600
        + paper
            .doi
            .as_ref()
            .map_or(0, |doi| if suspicious_doi(doi, paper) { 100 } else { 700 })
        + paper.openalex_id.is_some() as u32 * 300
}

fn canonical_source_score(paper: &AcademicPaper) -> u32 {
    paper
        .sources
        .iter()
        .map(|source| match source.provider.as_ref() {
            "semantic" => 900,
            "arxiv" => 800,
            "dblp" => 700,
            "openalex" => 250,
            "crossref" => 150,
            _ => 0,
        })
        .sum::<u32>()
        .min(2_400)
}

fn author_signal_score(paper: &AcademicPaper) -> u32 {
    (paper.authors.len().min(8) as u32) * 25
}

fn venue_signal_score(paper: &AcademicPaper) -> u32 {
    match paper.venue.as_deref().map(|v| v.to_ascii_lowercase()) {
        Some(venue)
            if venue.contains("arxiv")
                || venue.contains("neural information processing")
                || venue.contains("conference")
                || venue.contains("journal") =>
        {
            250
        }
        Some(_) => 120,
        None => 0,
    }
}

fn stable_year_score(paper: &AcademicPaper) -> u32 {
    match paper.year {
        Some(1900..=2026) => 200,
        Some(_) => 0,
        None => 50,
    }
}

fn citation_signal_score(paper: &AcademicPaper) -> u32 {
    let citations = paper.citation_count.unwrap_or(0).min(100_000);
    if citations == 0 {
        0
    } else {
        citations.ilog10() * 120 + citations.min(10_000) / 20
    }
}

fn suspicious_doi_penalty(paper: &AcademicPaper) -> u32 {
    paper
        .doi
        .as_ref()
        .filter(|doi| suspicious_doi(doi, paper))
        .map_or(0, |_| 0)
}

fn suspicious_doi(doi: &str, paper: &AcademicPaper) -> bool {
    let doi = doi.to_ascii_lowercase();
    let source_only_crossref_or_openalex = !paper.sources.is_empty()
        && paper.sources.iter().all(|source| {
            matches!(
                source.provider.as_ref(),
                "crossref" | "openalex" | "openalex_reference"
            )
        });
    doi.contains("10.65215")
        || (source_only_crossref_or_openalex
            && paper.semantic_scholar_id.is_none()
            && paper.arxiv_id.is_none()
            && paper.venue.is_none())
}

fn clean_citation_summary(mut summary: AcademicCitationSummary) -> AcademicCitationSummary {
    summary.citations = summary
        .citations
        .into_iter()
        .map(without_openalex_reference_sources)
        .collect();
    summary.references = summary
        .references
        .into_iter()
        .map(without_openalex_reference_sources)
        .collect();
    summary
}

fn prefer_institutional_locations(locations: Vec<FullTextLocation>) -> Vec<FullTextLocation> {
    let mut unique: Vec<FullTextLocation> = Vec::new();
    for location in locations {
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| existing.url == location.url)
        {
            if is_institutional_source(&location.source)
                && !is_institutional_source(&existing.source)
            {
                *existing = location;
            }
        } else {
            unique.push(location);
        }
    }
    unique
}

fn is_institutional_source(source: &str) -> bool {
    matches!(source, "ieee_institutional" | "acm_institutional")
}

fn material_links_for_paper(paper: &AcademicPaper) -> Vec<AcademicMaterialLink> {
    let mut materials = Vec::new();
    for (url, source) in [
        (paper.url.as_deref(), "paper_url"),
        (paper.pdf_url.as_deref(), "paper_pdf_url"),
    ] {
        if let Some(url) = url {
            materials.extend(material_links_from_url(url, source));
        }
    }
    if let Some(abstract_text) = &paper.abstract_text {
        materials.extend(material_links_from_text(abstract_text, "abstract"));
    }
    for source in &paper.sources {
        materials.extend(material_links_from_url(
            &source.url,
            source.provider.as_ref(),
        ));
    }
    merge_materials(Vec::new(), materials)
}

fn material_links_from_text(text: &str, source: &str) -> Vec<AcademicMaterialLink> {
    text.split_whitespace()
        .filter_map(|token| {
            let url = token
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\''
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | ','
                            | '.'
                            | ';'
                    )
                })
                .trim_end_matches('/');
            material_links_from_url(url, source).into_iter().next()
        })
        .collect()
}

fn material_links_from_url(url: &str, source: &str) -> Vec<AcademicMaterialLink> {
    let Some(kind) = classify_material_url(url) else {
        return Vec::new();
    };
    vec![AcademicMaterialLink {
        url: url.to_string(),
        kind: kind.to_string(),
        source: source.to_string(),
        confidence: material_confidence(kind).to_string(),
        title: None,
    }]
}

fn classify_material_url(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    if lower.contains("github.com/") || lower.contains("gitlab.com/") {
        return Some("code");
    }
    if lower.contains("huggingface.co/") {
        if lower.contains("/datasets/") {
            return Some("dataset");
        }
        if lower.contains("/spaces/") {
            return Some("demo");
        }
        return Some("model");
    }
    if lower.contains("paperswithcode.com/") {
        return Some("code");
    }
    if lower.contains("zenodo.org/") || lower.contains("figshare.com/") {
        return Some("dataset");
    }
    if lower.contains("arxiv.org/src/") || lower.contains("arxiv.org/e-print/") {
        return Some("supplement");
    }
    if lower.contains("colab.research.google.com/") {
        return Some("demo");
    }
    if lower.contains("docs.") || lower.contains("/docs") || lower.contains("readthedocs.io/") {
        return Some("documentation");
    }
    if lower.contains("project")
        || lower.contains("demo")
        || lower.contains("dataset")
        || lower.contains("code")
    {
        return Some("project");
    }
    None
}

fn material_confidence(kind: &str) -> &'static str {
    match kind {
        "code" | "dataset" | "model" | "demo" | "supplement" => "high",
        "documentation" => "medium",
        _ => "low",
    }
}

fn merge_materials(
    first: Vec<AcademicMaterialLink>,
    second: Vec<AcademicMaterialLink>,
) -> Vec<AcademicMaterialLink> {
    let mut out = Vec::new();
    for material in first.into_iter().chain(second) {
        if !out
            .iter()
            .any(|existing: &AcademicMaterialLink| existing.url.eq_ignore_ascii_case(&material.url))
        {
            out.push(material);
        }
    }
    out
}

async fn parse_pdf_bytes_with_timeout(
    bytes: Vec<u8>,
    format: String,
    limit: usize,
    options: Option<&AcademicParseOptions>,
    timeout: std::time::Duration,
    url: &str,
) -> Result<ParsedPdfDetails> {
    let url = url.to_string();
    let options = options.cloned();
    if timeout.is_zero() {
        return Err(GrokSearchError::Timeout(format!(
            "academic_read PDF parse timed out for {url}"
        )));
    }
    tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            parse_pdf_bytes_detailed(&bytes, &format, Some(limit), options.as_ref())
        }),
    )
    .await
    .map_err(|_| GrokSearchError::Timeout(format!("academic_read PDF parse timed out for {url}")))?
    .map_err(|err| GrokSearchError::Io(format!("academic_read parse task failed: {err}")))?
}

fn paper_matches_year_filter(
    paper: &AcademicPaper,
    year_from: Option<u32>,
    year_to: Option<u32>,
) -> bool {
    let Some(year) = paper.year else {
        return true;
    };
    year_from.map_or(true, |from| year >= from) && year_to.map_or(true, |to| year <= to)
}

fn search_result_is_relevant(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {}",
        paper.title,
        paper.abstract_text.as_deref().unwrap_or_default()
    );
    let haystack_tokens = meaningful_tokens(&haystack);
    let matches = matching_query_tokens(&query_tokens, &haystack_tokens);
    matches >= min_required_query_token_matches(query_tokens.len())
}

fn search_result_has_strong_overlap(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.len() <= 2 {
        return search_result_is_relevant(query, paper);
    }
    let haystack = format!(
        "{} {}",
        paper.title,
        paper.abstract_text.as_deref().unwrap_or_default()
    );
    let haystack_tokens = meaningful_tokens(&haystack);
    let matches = matching_query_tokens(&query_tokens, &haystack_tokens);
    matches >= strong_required_query_token_matches(query_tokens.len())
}

fn precise_search_result_is_relevant(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.is_empty() {
        return true;
    }
    let title_tokens = meaningful_tokens(&paper.title);
    let title_matches = matching_query_tokens(&query_tokens, &title_tokens);
    title_matches >= min_required_query_token_matches(query_tokens.len())
        || normalize_title(&paper.title).contains(&normalize_title(query))
}

fn rank_academic_results(query: &str, sort_by: AcademicSortBy, papers: &mut [AcademicPaper]) {
    let query_tokens = meaningful_tokens(query);
    papers.sort_by(|a, b| {
        academic_result_score(query, &query_tokens, sort_by, b).cmp(&academic_result_score(
            query,
            &query_tokens,
            sort_by,
            a,
        ))
    });
}

fn academic_result_score(
    query: &str,
    query_tokens: &[String],
    sort_by: AcademicSortBy,
    paper: &AcademicPaper,
) -> u32 {
    let title_tokens = meaningful_tokens(&paper.title);
    let abstract_tokens = meaningful_tokens(paper.abstract_text.as_deref().unwrap_or_default());
    let title_matches = matching_query_tokens(query_tokens, &title_tokens) as u32;
    let abstract_matches = matching_query_tokens(query_tokens, &abstract_tokens) as u32;
    let exact_title = (normalize_title(&paper.title) == normalize_title(query)) as u32;
    let pdf = paper.pdf_url.is_some() as u32;
    let oa = paper.open_access.unwrap_or(false) as u32;
    let citations = paper.citation_count.unwrap_or(0).min(10_000);
    let citation_score = if citations == 0 {
        0
    } else {
        citations.ilog10()
    };
    let citation_preference = match sort_by {
        AcademicSortBy::Citations => citation_score * 40 + citations.min(1_000) / 25,
        _ => citation_score,
    };
    let date_preference = match sort_by {
        AcademicSortBy::Date => paper.year.unwrap_or(0).saturating_sub(1900).min(200),
        _ => 0,
    };

    exact_title * 1_000
        + title_matches * 100
        + abstract_matches * 20
        + citation_preference
        + date_preference
        + pdf * 3
        + oa
}

fn matching_query_tokens(query_tokens: &[String], haystack_tokens: &[String]) -> usize {
    query_tokens
        .iter()
        .filter(|token| haystack_tokens.iter().any(|candidate| candidate == *token))
        .count()
}

fn min_required_query_token_matches(query_token_count: usize) -> usize {
    if query_token_count <= 2 {
        1
    } else {
        2
    }
}

fn strong_required_query_token_matches(query_token_count: usize) -> usize {
    query_token_count.min(3)
}

fn meaningful_tokens(text: &str) -> Vec<String> {
    normalize_title(text)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !ACADEMIC_STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

const ACADEMIC_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "for", "from", "how", "into", "not", "of", "on", "or", "the", "this",
    "to", "with", "paper",
];

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
    use crate::providers::{
        parse_dblp_search, parse_openalex_work, parse_semantic_paper,
        without_openalex_reference_sources,
    };
    use serde_json::json;

    #[test]
    fn identifier_normalizes_common_academic_ids() {
        assert_eq!(
            parse_identifier("https://arxiv.org/pdf/1706.03762.pdf"),
            Identifier::Arxiv("1706.03762".to_string())
        );
        assert_eq!(
            parse_identifier("10.48550/arXiv.1706.03762"),
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
        assert!(paper
            .sources
            .iter()
            .any(|source| source.provider.as_ref() == "openalex_reference"));
        let search_paper = without_openalex_reference_sources(paper);
        assert!(!search_paper
            .sources
            .iter()
            .any(|source| source.provider.as_ref() == "openalex_reference"));
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

    #[tokio::test]
    async fn academic_search_zero_max_results_returns_empty_without_providers() {
        let service = AcademicService::new(
            reqwest::Client::new(),
            Config::from_env_map(Vec::<(String, String)>::new()),
        );
        let output = service
            .search(AcademicSearchInput {
                query: "transformer".into(),
                max_results: Some(0),
                ..Default::default()
            })
            .await
            .expect("zero max_results should be valid");
        assert_eq!(output.papers_count, 0);
        assert!(output.papers.is_empty());
        assert!(output.sources_used.is_empty());
    }

    #[test]
    fn title_like_get_rejects_dblp_near_miss() {
        let id = Identifier::Query("Attention Is All You Need".into());
        let near_miss = AcademicPaper {
            title:
                "Attentional Transfer is All You Need: Technology-aware Layout Pattern Generation."
                    .into(),
            ..Default::default()
        };
        let exact = AcademicPaper {
            title: "Attention Is All You Need".into(),
            ..Default::default()
        };
        assert!(!resolved_paper_matches_identifier(&id, &near_miss));
        assert!(resolved_paper_matches_identifier(&id, &exact));
    }

    #[test]
    fn nonsense_query_filters_unrelated_papers() {
        let paper = AcademicPaper {
            title: "Spectroscopic Needs for Calibration of LSST Photometric Redshifts".into(),
            abstract_text: Some("Dark energy survey calibration".into()),
            ..Default::default()
        };
        assert!(!search_result_is_relevant(
            "zzzxxy nonexistent paper qwertyuiopasdf",
            &paper
        ));
        assert!(search_result_is_relevant(
            "photometric redshifts calibration",
            &paper
        ));
    }

    #[test]
    fn academic_search_modes_select_expected_default_sources() {
        assert_eq!(
            selected_sources(&[], AcademicSearchMode::Balanced).expect("default sources"),
            vec!["dblp", "semantic", "arxiv"]
        );
        assert_eq!(
            selected_sources(&[], AcademicSearchMode::Precise).expect("default sources"),
            vec!["dblp", "semantic", "arxiv"]
        );
        assert_eq!(
            selected_sources(&[], AcademicSearchMode::Broad).expect("default sources"),
            vec!["dblp", "semantic", "arxiv", "openalex", "crossref"]
        );
    }

    #[test]
    fn selected_sources_accept_common_semantic_scholar_aliases() {
        assert_eq!(
            selected_sources(
                &["semantic_scholar".into(), "semanticscholar,s2".into()],
                AcademicSearchMode::Balanced
            )
            .expect("aliases should normalize"),
            vec!["semantic", "semantic", "semantic"]
        );
    }

    #[test]
    fn selected_sources_reject_unknown_values() {
        let err = selected_sources(
            &["semantic_scholar".into(), "scholar".into()],
            AcademicSearchMode::Balanced,
        )
        .expect_err("unknown source should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn citation_identifiers_prefer_provider_native_ids() {
        let paper = AcademicPaper {
            title: "Attention Is All You Need".into(),
            doi: Some("10.48550/arXiv.1706.03762".into()),
            arxiv_id: Some("1706.03762".into()),
            semantic_scholar_id: Some("semantic-id".into()),
            openalex_id: Some("https://openalex.org/W123".into()),
            ..Default::default()
        };
        assert_eq!(
            citation_identifiers_for_paper(&paper),
            vec![
                Identifier::Semantic("semantic-id".into()),
                Identifier::OpenAlex("https://openalex.org/W123".into()),
                Identifier::Doi("10.48550/arXiv.1706.03762".into()),
                Identifier::Arxiv("1706.03762".into()),
            ]
        );
    }

    #[test]
    fn academic_search_mode_rejects_unknown_values() {
        let err = search_mode(Some("exploratory")).expect_err("unknown mode should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn academic_sort_by_rejects_unknown_values() {
        assert_eq!(
            academic_sort_by(Some("citations")).expect("valid sort"),
            AcademicSortBy::Citations
        );
        let err = academic_sort_by(Some("impact")).expect_err("unknown sort should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn precise_relevance_requires_title_overlap() {
        let abstract_only = AcademicPaper {
            title: "Generic Systems Paper".into(),
            abstract_text: Some("large language model evaluation".into()),
            ..Default::default()
        };
        let title_match = AcademicPaper {
            title: "A Survey on Evaluation of Large Language Models".into(),
            ..Default::default()
        };
        assert!(!precise_search_result_is_relevant(
            "large language model evaluation",
            &abstract_only
        ));
        assert!(precise_search_result_is_relevant(
            "large language model evaluation",
            &title_match
        ));
    }

    #[test]
    fn strong_overlap_rejects_sort_boosted_partial_matches() {
        let partial = AcademicPaper {
            title: "A comprehensive survey of loss functions and metrics in deep learning".into(),
            abstract_text: Some("survey methods for deep learning".into()),
            ..Default::default()
        };
        let relevant = AcademicPaper {
            title: "Retrieval-Augmented Generation for Large Language Models: A Survey".into(),
            ..Default::default()
        };
        assert!(!search_result_has_strong_overlap(
            "retrieval augmented generation survey",
            &partial
        ));
        assert!(search_result_has_strong_overlap(
            "retrieval augmented generation survey",
            &relevant
        ));
    }

    #[test]
    fn multi_token_relevance_rejects_single_generic_token_match() {
        let generic = AcademicPaper {
            title: "R: A Language and Environment for Statistical Computing".into(),
            abstract_text: Some("A statistical programming environment".into()),
            ..Default::default()
        };
        let relevant = AcademicPaper {
            title: "A Survey on Evaluation of Large Language Models".into(),
            abstract_text: Some("Evaluation methods for large language model systems".into()),
            ..Default::default()
        };
        assert!(!search_result_is_relevant(
            "large language model evaluation",
            &generic
        ));
        assert!(search_result_is_relevant(
            "large language model evaluation",
            &relevant
        ));
    }

    #[test]
    fn academic_ranking_prioritizes_exact_title_then_overlap() {
        let mut papers = vec![
            AcademicPaper {
                title: "Large Models in General".into(),
                abstract_text: Some("large language model evaluation".into()),
                citation_count: Some(10_000),
                ..Default::default()
            },
            AcademicPaper {
                title: "A Survey on Evaluation of Large Language Models".into(),
                citation_count: Some(10),
                ..Default::default()
            },
        ];
        rank_academic_results(
            "A Survey on Evaluation of Large Language Models",
            AcademicSortBy::Relevance,
            &mut papers,
        );
        assert_eq!(
            papers[0].title,
            "A Survey on Evaluation of Large Language Models"
        );
    }

    #[test]
    fn citation_sort_boosts_cited_relevant_papers_without_beating_exact_title() {
        let mut papers = vec![
            AcademicPaper {
                title: "Large Language Model Evaluation Notes".into(),
                citation_count: Some(5),
                ..Default::default()
            },
            AcademicPaper {
                title: "Large Language Model Evaluation Survey".into(),
                citation_count: Some(5_000),
                ..Default::default()
            },
            AcademicPaper {
                title: "Large Language Model Evaluation".into(),
                citation_count: Some(1),
                ..Default::default()
            },
        ];
        rank_academic_results(
            "Large Language Model Evaluation",
            AcademicSortBy::Citations,
            &mut papers,
        );
        assert_eq!(papers[0].title, "Large Language Model Evaluation");
        assert_eq!(papers[1].title, "Large Language Model Evaluation Survey");
    }

    #[test]
    fn year_filter_keeps_unknown_years_and_bounds_known_years() {
        let unknown = AcademicPaper {
            title: "Unknown".into(),
            year: None,
            ..Default::default()
        };
        let old = AcademicPaper {
            title: "Old".into(),
            year: Some(2023),
            ..Default::default()
        };
        let current = AcademicPaper {
            title: "Current".into(),
            year: Some(2024),
            ..Default::default()
        };
        let future = AcademicPaper {
            title: "Future".into(),
            year: Some(2025),
            ..Default::default()
        };
        assert!(paper_matches_year_filter(&unknown, Some(2024), Some(2024)));
        assert!(!paper_matches_year_filter(&old, Some(2024), Some(2024)));
        assert!(paper_matches_year_filter(&current, Some(2024), Some(2024)));
        assert!(!paper_matches_year_filter(&future, Some(2024), Some(2024)));
    }

    #[test]
    fn title_query_fallback_selector_requires_exact_normalized_title() {
        let exact = AcademicPaper {
            title: "Attention Is All You Need".into(),
            ..Default::default()
        };
        let near_miss = AcademicPaper {
            title: "Attention Is Almost All You Need".into(),
            ..Default::default()
        };
        let found = select_best_title_match(
            "attention is all you need",
            vec![near_miss.clone(), exact.clone()],
        )
        .expect("exact title");
        assert_eq!(found.title, exact.title);
        assert!(select_best_title_match("attention is all you need", vec![near_miss]).is_none());
    }

    #[test]
    fn title_query_selector_prefers_canonical_scholarly_metadata() {
        let query = "Canonical Systems Paper";
        let low_confidence = AcademicPaper {
            title: query.into(),
            year: Some(2025),
            doi: Some("10.65215/example".into()),
            sources: vec![
                Source::new("https://openalex.org/W1", "openalex"),
                Source::new("https://doi.org/10.65215/example", "crossref"),
            ],
            ..Default::default()
        };
        let canonical = AcademicPaper {
            title: query.into(),
            authors: vec!["Ada Lovelace".into(), "Grace Hopper".into()],
            year: Some(2017),
            venue: Some("Conference on Systems".into()),
            arxiv_id: Some("1701.00001".into()),
            semantic_scholar_id: Some("semantic-paper".into()),
            citation_count: Some(10_000),
            sources: vec![
                Source::new(
                    "https://semanticscholar.org/paper/semantic-paper",
                    "semantic",
                ),
                Source::new("https://arxiv.org/abs/1701.00001", "arxiv"),
            ],
            ..Default::default()
        };
        let found = select_best_title_match(query, vec![low_confidence, canonical.clone()])
            .expect("canonical match");
        assert_eq!(found.semantic_scholar_id, canonical.semantic_scholar_id);
    }

    #[test]
    fn title_query_selector_rejects_near_title_even_when_highly_cited() {
        let near = AcademicPaper {
            title: "Canonical Systems Paper Extended".into(),
            citation_count: Some(100_000),
            semantic_scholar_id: Some("near".into()),
            sources: vec![Source::new(
                "https://semanticscholar.org/paper/near",
                "semantic",
            )],
            ..Default::default()
        };
        assert!(select_best_title_match("Canonical Systems Paper", vec![near]).is_none());
    }

    #[test]
    fn title_query_selector_allows_low_confidence_provider_when_only_exact_candidate() {
        let query = "Niche Exact Paper";
        let crossref_only = AcademicPaper {
            title: query.into(),
            year: Some(2024),
            doi: Some("10.1234/niche".into()),
            sources: vec![Source::new("https://doi.org/10.1234/niche", "crossref")],
            ..Default::default()
        };
        let found = select_best_title_match(query, vec![crossref_only.clone()])
            .expect("single exact candidate should still be usable");
        assert_eq!(found.doi, crossref_only.doi);
    }

    #[test]
    fn pdf_locator_validation_requires_one_location() {
        assert!(ensure_valid_locator(
            &AcademicPdfLocator {
                pdf_url: Some("https://example.com/paper.pdf".to_string()),
                ..Default::default()
            },
            "academic_pdf_read"
        )
        .is_ok());

        let err = ensure_valid_locator(&AcademicPdfLocator::default(), "academic_pdf_read")
            .expect_err("missing locator should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));

        let err = ensure_valid_locator(
            &AcademicPdfLocator {
                identifier: Some("arXiv:1706.03762".to_string()),
                pdf_url: Some("https://example.com/paper.pdf".to_string()),
                ..Default::default()
            },
            "academic_pdf_read",
        )
        .expect_err("multiple locators should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn pdf_structure_profiles_map_to_llm_options() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL", "MiniMax-M3"),
            ("GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE", "strict"),
        ]);

        let strict = llm_options_for_structure(
            &AcademicPdfStructureInput {
                locator: AcademicPdfLocator {
                    pdf_url: Some("https://example.com/paper.pdf".to_string()),
                    ..Default::default()
                },
                cache_policy: Some(AcademicPdfCachePolicy::Refresh),
                ..Default::default()
            },
            &config,
        );
        assert_eq!(strict.enabled, Some(true));
        assert_eq!(strict.model.as_deref(), Some("MiniMax-M3"));
        assert_eq!(strict.max_chunk_chars, Some(5_500));
        assert_eq!(strict.overlap_chars, Some(700));
        assert_eq!(strict.concurrency, Some(1));
        assert_eq!(strict.max_output_tokens, Some(1_800));
        assert_eq!(strict.cache_enabled, Some(true));
        assert_eq!(strict.cache_refresh, Some(true));

        let fast_bypass = llm_options_for_structure(
            &AcademicPdfStructureInput {
                locator: AcademicPdfLocator {
                    pdf_url: Some("https://example.com/paper.pdf".to_string()),
                    ..Default::default()
                },
                profile: Some(AcademicPdfStructureProfile::Fast),
                cache_policy: Some(AcademicPdfCachePolicy::Bypass),
                model: Some("custom-model".to_string()),
                ..Default::default()
            },
            &config,
        );
        assert_eq!(fast_bypass.model.as_deref(), Some("custom-model"));
        assert_eq!(fast_bypass.max_chunk_chars, Some(4_500));
        assert_eq!(fast_bypass.cache_enabled, Some(false));
        assert_eq!(fast_bypass.cache_refresh, Some(false));
    }

    #[test]
    fn pdf_cache_key_uses_hash_without_raw_url() {
        let location = FullTextLocation {
            url: "https://example.com/paper.pdf?token=secret".to_string(),
            source: "direct_url".to_string(),
            status: "direct_url".to_string(),
        };
        let key = pdf_cache_key(&location, 1024);
        assert!(key.starts_with("academic_pdf:v1:"));
        assert!(!key.contains("token=secret"));
        assert!(!key.contains("paper.pdf"));
    }

    #[test]
    fn pdf_download_retry_policy_covers_timeout_and_5xx() {
        assert_eq!(pdf_download_retry_delay_ms(1), 600);
        assert_eq!(pdf_download_retry_delay_ms(2), 1_200);
        assert!(is_retryable_pdf_download_error(&GrokSearchError::Timeout(
            "slow".into()
        )));
        assert!(is_retryable_pdf_download_error(&GrokSearchError::Upstream(
            "academic pdf returned HTTP 503".into()
        )));
        assert!(!is_retryable_pdf_download_error(
            &GrokSearchError::InvalidParams("bad".into())
        ));
    }

    #[test]
    fn canonical_merge_starts_from_best_candidate() {
        let weak = AcademicPaper {
            title: "Same Paper".into(),
            doi: Some("10.65215/weak".into()),
            sources: vec![Source::new("https://doi.org/10.65215/weak", "crossref")],
            ..Default::default()
        };
        let strong = AcademicPaper {
            title: "Same Paper".into(),
            semantic_scholar_id: Some("sem".into()),
            arxiv_id: Some("2401.00001".into()),
            citation_count: Some(500),
            sources: vec![Source::new(
                "https://semanticscholar.org/paper/sem",
                "semantic",
            )],
            ..Default::default()
        };
        let merged = merge_canonical_candidates(vec![weak, strong]);
        assert_eq!(merged.semantic_scholar_id.as_deref(), Some("sem"));
        assert_eq!(merged.arxiv_id.as_deref(), Some("2401.00001"));
    }

    #[test]
    fn citation_summary_cleanup_removes_openalex_reference_sources() {
        let relation = AcademicPaper {
            title: "Related".into(),
            sources: vec![
                Source::new("https://openalex.org/W0", "openalex"),
                Source::new("https://openalex.org/W1", "openalex_reference"),
            ],
            ..Default::default()
        };
        let cleaned = clean_citation_summary(AcademicCitationSummary {
            citations: vec![relation.clone()],
            references: vec![relation],
        });
        assert!(cleaned.citations[0]
            .sources
            .iter()
            .all(|source| source.provider.as_ref() != "openalex_reference"));
        assert!(cleaned.references[0]
            .sources
            .iter()
            .all(|source| source.provider.as_ref() != "openalex_reference"));
    }

    #[tokio::test]
    async fn academic_read_download_timeout_returns_tool_error_promptly() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::thread;
        use std::time::Duration as StdDuration;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}/slow.pdf", listener.local_addr().unwrap());
        let _handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            thread::sleep(StdDuration::from_millis(500));
        });

        let mut config = Config::from_env_map([
            ("GROK_SEARCH_TIMEOUT_SECONDS", "60"),
            ("GROK_SEARCH_ACADEMIC_PDF_CACHE_ENABLED", "false"),
        ]);
        config.timeout = std::time::Duration::from_millis(50);
        let service = AcademicService::new(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
            config,
        );
        let err = service
            .read(None, Some(url), Some(10), Some("text".to_string()), None)
            .await
            .expect_err("download should time out");
        assert!(
            matches!(err, GrokSearchError::Timeout(_)),
            "expected timeout, got {err:?}"
        );
    }

    #[tokio::test]
    async fn pdf_download_uses_cache_on_second_call() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let _handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            requests_for_thread.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let body = b"%PDF-cache-test";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });

        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config::from_env_map([
            (
                "GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH",
                dir.path()
                    .join("pdf-cache.redb")
                    .to_string_lossy()
                    .to_string(),
            ),
            ("GROK_SEARCH_TIMEOUT_SECONDS", "5".to_string()),
        ]);
        let service = AcademicService::new(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
            config,
        );
        let location = FullTextLocation {
            url,
            source: "direct_url".to_string(),
            status: "direct_url".to_string(),
        };

        let first = service
            .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
            .await
            .expect("first download");
        let second = service
            .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
            .await
            .expect("second download");

        assert!(!first.cache.hit);
        assert!(first.cache.stored);
        assert!(second.cache.hit);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pdf_download_refresh_bypasses_existing_cache() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let _handle = thread::spawn(move || {
            for idx in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let body = if idx == 0 {
                    b"%PDF-cache-first".as_slice()
                } else {
                    b"%PDF-cache-refresh".as_slice()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config::from_env_map([
            (
                "GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH",
                dir.path()
                    .join("pdf-cache.redb")
                    .to_string_lossy()
                    .to_string(),
            ),
            ("GROK_SEARCH_TIMEOUT_SECONDS", "5".to_string()),
        ]);
        let service = AcademicService::new(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
            config,
        );
        let location = FullTextLocation {
            url,
            source: "direct_url".to_string(),
            status: "direct_url".to_string(),
        };

        let first = service
            .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
            .await
            .expect("first download");
        let refreshed = service
            .download_pdf_for_location(&location, AcademicPdfCachePolicy::Refresh)
            .await
            .expect("refresh download");

        assert_eq!(first.bytes, b"%PDF-cache-first");
        assert_eq!(refreshed.bytes, b"%PDF-cache-refresh");
        assert!(!refreshed.cache.hit);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn academic_read_parse_timeout_is_mapped_to_timeout_error() {
        let err = match parse_pdf_bytes_with_timeout(
            b"%PDF-1.7\n".to_vec(),
            "text".to_string(),
            10,
            None,
            std::time::Duration::from_secs(0),
            "https://example.com/paper.pdf",
        )
        .await
        {
            Ok(_) => panic!("parse should time out before the blocking task completes"),
            Err(err) => err,
        };
        assert!(
            matches!(err, GrokSearchError::Timeout(_)),
            "expected timeout, got {err:?}"
        );
    }

    #[tokio::test]
    async fn academic_read_rejects_invalid_output_format_before_fetching() {
        let service = AcademicService::new(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
            Config::from_env_map(Vec::<(String, String)>::new()),
        );
        let err = service
            .read(
                None,
                Some("http://127.0.0.1:1/paper.pdf".to_string()),
                Some(10),
                Some("html".to_string()),
                None,
            )
            .await
            .expect_err("invalid format should fail before network fetch");
        assert!(
            matches!(err, GrokSearchError::InvalidParams(_)),
            "expected invalid params, got {err:?}"
        );
    }

    #[test]
    fn material_links_detect_common_research_artifacts() {
        let text = "Code: https://github.com/example/repo Dataset https://huggingface.co/datasets/org/data Demo https://huggingface.co/spaces/org/demo";
        let links = material_links_from_text(text, "abstract");
        let kinds: Vec<_> = links.iter().map(|link| link.kind.as_str()).collect();
        assert_eq!(kinds, vec!["code", "dataset", "demo"]);
        assert!(links.iter().all(|link| link.confidence == "high"));
    }

    #[tokio::test]
    async fn academic_read_fulltext_locations_include_deduped_fallback_candidates() {
        let service = AcademicService::new(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("test client"),
            Config::from_env_map(Vec::<(String, String)>::new()),
        );
        let paper = AcademicPaper {
            title: "Paper".into(),
            arxiv_id: Some("2401.00001".into()),
            pdf_url: Some("https://arxiv.org/pdf/2401.00001".into()),
            ..Default::default()
        };
        let locations = service
            .resolve_fulltext_locations(&paper)
            .await
            .expect("locations");
        assert_eq!(
            locations
                .iter()
                .filter(|location| location.url == "https://arxiv.org/pdf/2401.00001")
                .count(),
            1
        );
        assert!(locations
            .iter()
            .any(|location| location.source == "paper" || location.source == "arxiv"));
    }
}
