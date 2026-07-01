use std::collections::BTreeMap;
use std::time::Instant;

use async_trait::async_trait;
use grok_search_cache::{PdfCachePut, PdfCacheStore, RedbPdfCache};
use grok_search_config::Config;
use grok_search_parse::{
    parse_academic_identifier as parse_identifier, rrf_merge_papers as rrf_merge,
};
use grok_search_pdf::{
    download_pdf_bytes_optimized, OptimizedPdfDownloadOptions, ParsedPdfDetails,
};
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, AcademicServiceProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicCitationsOutput, AcademicDownloadPdfOutput, AcademicGetOutput,
    AcademicMaterialLink, AcademicPaper, AcademicParseOptions, AcademicParsePdfOutput,
    AcademicPdfArtifactsInput, AcademicPdfArtifactsOutput, AcademicPdfCacheInfo,
    AcademicPdfCachePolicy, AcademicPdfDownloadInput, AcademicPdfDownloadOutput,
    AcademicPdfLocator, AcademicPdfReadInput, AcademicPdfReadOutput, AcademicPdfStructureInput,
    AcademicPdfStructureOutput, AcademicProgressiveGetInput, AcademicProgressiveGetOutput,
    AcademicProgressivePaper, AcademicReadOutput, AcademicSearchInput, AcademicSearchOutput,
    GrokSearchError, Result, Source,
};

use crate::institutional::InstitutionalAccessManager;
use crate::llm_artifacts;
use crate::llm_progressive;
use crate::providers::{
    without_openalex_reference_sources, ArxivProvider, CrossrefProvider, DblpProvider,
    OpenAlexProvider, SciHubProvider, SemanticProvider, UnpaywallProvider,
};

pub(crate) const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";
const DEFAULT_SOURCES: &[&str] = &["dblp", "semantic", "arxiv"];
const ALL_SOURCES: &[&str] = &["dblp", "semantic", "arxiv", "openalex", "crossref"];
mod identifiers;
mod materials;
mod pdf;
mod ranking;
mod search_options;
mod structure;

#[cfg(test)]
mod tests;

use self::identifiers::*;
use self::materials::*;
use self::pdf::*;
use self::ranking::*;
use self::search_options::*;
use self::structure::*;

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
        let vision_config =
            llm_artifacts::ArtifactVisionRunConfig::from_input(&input, &self.config)?;
        let parse_options = AcademicParseOptions {
            images_dir: input.images_dir.clone(),
            tables_dir: input.tables_dir.clone(),
            extract_images: input.extract_images,
            extract_tables: input.extract_tables,
            text_processing_mode: input.text_mode.clone(),
            vision_profile: vision_config
                .enabled()
                .then_some(vision_config.profile.clone()),
            vision_max_pages: Some(vision_config.max_pages),
            vision_render_dpi: Some(vision_config.render_dpi),
            ..Default::default()
        };
        let details = self
            .read_pdf_details_from_locator(
                input.locator.clone(),
                input.max_chars,
                Some("markdown".to_string()),
                Some(parse_options),
                input.cache_policy.unwrap_or_default(),
                "academic_pdf_artifacts",
            )
            .await?;
        let mut vision = if vision_config.enabled() {
            llm_artifacts::run_artifact_micro(&details.parsed, &input, &self.config, &self.client)
                .await?
        } else {
            None
        };
        let mut refined_artifacts = Vec::new();
        if let Some(vision) = vision.as_mut() {
            refined_artifacts.extend(llm_artifacts::write_refined_completion_artifacts(
                &details.parsed,
                &input,
                vision,
            )?);
        }
        let mut artifacts = details.parsed.artifacts;
        artifacts.extend(refined_artifacts);
        Ok(AcademicPdfArtifactsOutput {
            identifier: details.identifier,
            url: details.url,
            pdf_url: details.pdf_url,
            source: details.source,
            fulltext_status: details.fulltext_status,
            resolver_chain: details.resolver_chain,
            artifacts,
            parse_capabilities: details.parsed.capabilities,
            processing: details.parsed.processing,
            pdf_cache: Some(details.pdf_cache),
            vision,
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
            download.bytes.clone(),
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
