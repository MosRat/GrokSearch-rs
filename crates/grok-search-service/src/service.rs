use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::cache::SourceCache;
use crate::domain_filter::filter_sources_by_domains;
use crate::logging::DebugLogger;
use crate::response_budget::apply_response_budget;
use grok_search_config::Config;
use grok_search_net::proxy::ProxyDiagnostics;
pub use grok_search_provider_core::{
    AcademicServiceProvider, AiProvider, SourceProvider, WechatProvider, ZhihuProvider,
};
use grok_search_source_core::{SourceCaps, SourceRouter};
use grok_search_types::model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchResponse, SearchTool,
};
use grok_search_types::model::source::{merge_sources, Source};
use grok_search_types::model::tool::{GetSourcesOutput, WebSearchInput, WebSearchOutput};
use grok_search_types::{
    AcademicCitationsOutput, AcademicDownloadPdfOutput, AcademicGetOutput, AcademicParseOptions,
    AcademicParsePdfOutput, AcademicPdfArtifactsInput, AcademicPdfArtifactsOutput,
    AcademicPdfDownloadInput, AcademicPdfDownloadOutput, AcademicPdfReadInput,
    AcademicPdfReadOutput, AcademicPdfStructureInput, AcademicPdfStructureOutput,
    AcademicProgressiveGetInput, AcademicProgressiveGetOutput, AcademicReadOutput,
    AcademicSearchInput, AcademicSearchOutput, WechatSearchInput, WechatSearchOutput,
    ZhihuSearchInput, ZhihuSearchOutput,
};
use grok_search_types::{GrokSearchError, Result};

#[derive(Clone)]
pub struct SearchService {
    pub(crate) config: Config,
    pub(crate) ai: Arc<dyn AiProvider>,
    /// Model name written into every `SearchRequest` produced by the service.
    /// Resolved once from `config` at construction so each transport gets the
    /// model it actually understands: `grok_model` for Responses, and
    /// `openai_compatible_model` (falling back to `grok_model`) for the
    /// chat-completions transport. Per-call overrides via `WebSearchInput.model`
    /// still win.
    pub(crate) default_model: String,
    pub(crate) sources: Option<Arc<dyn SourceProvider>>,
    pub(crate) fallback_sources: Option<Arc<dyn SourceProvider>>,
    pub(crate) cache: Arc<Mutex<SourceCache>>,
    /// Shared reqwest client for the sources pipeline (same instance handed to
    /// providers). Stored here because resolve_content needs direct GET access.
    pub(crate) http_client: reqwest::Client,
    /// Specialist extractor router. Empty in Phase 1. Behind `Arc` so
    /// `SearchService: Clone` still holds (the router is not `Clone`).
    pub(crate) source_router: Arc<SourceRouter>,
    pub(crate) proxy_diagnostics: ProxyDiagnostics,
    pub(crate) academic: Option<Arc<dyn AcademicServiceProvider>>,
    pub(crate) wechat: Option<Arc<dyn WechatProvider>>,
    pub(crate) zhihu: Option<Arc<dyn ZhihuProvider>>,
    pub(crate) logger: DebugLogger,
}

pub struct SearchServiceParts {
    pub config: Config,
    pub ai: Arc<dyn AiProvider>,
    pub sources: Option<Arc<dyn SourceProvider>>,
    pub fallback_sources: Option<Arc<dyn SourceProvider>>,
    pub http_client: reqwest::Client,
    pub source_router: SourceRouter,
    pub proxy_diagnostics: ProxyDiagnostics,
    pub academic: Option<Arc<dyn AcademicServiceProvider>>,
    pub wechat: Option<Arc<dyn WechatProvider>>,
    pub zhihu: Option<Arc<dyn ZhihuProvider>>,
}

impl SearchService {
    pub fn from_parts(parts: SearchServiceParts) -> Self {
        let config = parts.config;
        Self {
            cache: Arc::new(Mutex::new(SourceCache::new(config.cache_size))),
            default_model: resolve_default_model(&config),
            config: config.clone(),
            ai: parts.ai,
            sources: parts.sources,
            fallback_sources: parts.fallback_sources,
            http_client: parts.http_client,
            source_router: Arc::new(parts.source_router),
            proxy_diagnostics: parts.proxy_diagnostics,
            academic: parts.academic,
            wechat: parts.wechat,
            zhihu: parts.zhihu,
            logger: DebugLogger::new(config.debug_log_path.clone()),
        }
    }

    pub fn fake_with_sources() -> Self {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "fake-grok"),
            ("TAVILY_API_KEY", "fake-tavily"),
        ]);
        Self {
            cache: Arc::new(Mutex::new(SourceCache::new(256))),
            default_model: resolve_default_model(&config),
            config: config.clone(),
            ai: Arc::new(FakeAiProvider),
            sources: Some(Arc::new(FakeSourceProvider)),
            fallback_sources: None,
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::new(config.debug_log_path.clone()),
        }
    }

    /// Unified test factory: override AI / primary / fallback providers and
    /// inject extra env vars. Use `fake_with_sources()` for the trivial case.
    pub fn fake_custom<I, K, V>(
        ai: Option<Arc<dyn AiProvider>>,
        primary: Arc<dyn SourceProvider>,
        fallback: Option<Arc<dyn SourceProvider>>,
        overrides: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut vars = vec![
            ("GROK_SEARCH_API_KEY".to_string(), "fake-grok".to_string()),
            ("TAVILY_API_KEY".to_string(), "fake-tavily".to_string()),
        ];
        if fallback.is_some() {
            vars.push((
                "FIRECRAWL_API_KEY".to_string(),
                "fake-firecrawl".to_string(),
            ));
        }
        vars.extend(
            overrides
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        let config = Config::from_env_map(vars);

        Self {
            cache: Arc::new(Mutex::new(SourceCache::new(256))),
            default_model: resolve_default_model(&config),
            config: config.clone(),
            ai: ai.unwrap_or_else(|| Arc::new(FakeAiProvider)),
            sources: Some(primary),
            fallback_sources: fallback,
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::new(config.debug_log_path.clone()),
        }
    }

    /// Test factory that injects a populated [`SourceRouter`] so
    /// fallback behavior can be exercised with fake extractors. Mirrors
    /// `fake_custom`'s provider wiring.
    pub fn fake_with_router(
        primary: Arc<dyn SourceProvider>,
        fallback: Option<Arc<dyn SourceProvider>>,
        router: SourceRouter,
    ) -> Self {
        let mut vars = vec![
            ("GROK_SEARCH_API_KEY".to_string(), "fake-grok".to_string()),
            ("TAVILY_API_KEY".to_string(), "fake-tavily".to_string()),
        ];
        if fallback.is_some() {
            vars.push((
                "FIRECRAWL_API_KEY".to_string(),
                "fake-firecrawl".to_string(),
            ));
        }
        let config = Config::from_env_map(vars);
        Self {
            cache: Arc::new(Mutex::new(SourceCache::new(256))),
            default_model: resolve_default_model(&config),
            config: config.clone(),
            ai: Arc::new(FakeAiProvider),
            sources: Some(primary),
            fallback_sources: fallback,
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(router),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::new(config.debug_log_path.clone()),
        }
    }

    pub fn fake_with_wechat(wechat: Arc<dyn WechatProvider>) -> Self {
        let mut service = Self::fake_with_sources();
        service.wechat = Some(wechat);
        service
    }

    pub fn fake_with_zhihu(zhihu: Arc<dyn ZhihuProvider>) -> Self {
        let mut service = Self::fake_with_sources();
        service.zhihu = Some(zhihu);
        service
    }

    pub async fn web_search(&self, input: WebSearchInput) -> Result<WebSearchOutput> {
        let op_start = Instant::now();
        let request_id = self.logger.request_id();
        self.logger.event(
            &request_id,
            "debug",
            "web_search.start",
            Some("web_search"),
            None,
            json!({
                "query_chars": input.query.chars().count(),
                "extra_sources": input.extra_sources,
                "recency_days": input.recency_days,
                "include_domains": input.include_domains,
                "exclude_domains": input.exclude_domains,
                "include_content": input.include_content,
                "response_format": input.response_format,
            }),
        );
        let result = self.web_search_inner(input).await;
        match &result {
            Ok(output) => self.logger.event(
                &request_id,
                "debug",
                "web_search.success",
                Some("web_search"),
                Some(op_start.elapsed()),
                json!({
                    "session_id": output.session_id,
                    "sources_count": output.sources_count,
                    "fallback_used": output.fallback_used,
                    "fallback_reason": output.fallback_reason,
                    "truncated": output.truncated,
                }),
            ),
            Err(err) => self.logger.error(
                &request_id,
                "web_search.error",
                Some("web_search"),
                Some(op_start.elapsed()),
                err,
                json!({}),
            ),
        }
        result
    }

    async fn web_search_inner(&self, input: WebSearchInput) -> Result<WebSearchOutput> {
        // D-02: single global deadline shared by Grok + supplemental fetch + enrichment.
        let deadline = tokio::time::Instant::now() + self.config.timeout;
        // response_format (Anthropic tool-design guidance: concise|detailed)
        // wins over the legacy include_content flag when both are present.
        let format_include_content = match input.response_format.as_deref() {
            None => None,
            Some("concise") => Some(false),
            Some("detailed") => Some(true),
            Some(other) => {
                return Err(GrokSearchError::InvalidParams(format!(
                    "response_format must be \"concise\" or \"detailed\", got \"{other}\""
                )))
            }
        };
        let include_content =
            format_include_content.unwrap_or_else(|| input.include_content.unwrap_or(true));

        let mut uuid_buf = [0u8; uuid::fmt::Simple::LENGTH];
        let session_id = {
            let encoded = Uuid::new_v4().simple().encode_lower(&mut uuid_buf);
            encoded[..12].to_string()
        };
        let effective_extra_sources = input
            .extra_sources
            .unwrap_or(self.config.default_extra_sources);

        let filters = SearchFilters {
            recency_days: input.recency_days,
            include_domains: input.include_domains.clone(),
            exclude_domains: input.exclude_domains.clone(),
        };

        // Speculative fan-out: fetch enough sources to satisfy whichever path
        // (enrichment or fallback) the Grok response routes us into. The
        // speculative call fires concurrently with Grok via tokio::join!, so
        // total latency is roughly max(Grok, Tavily) instead of the sum. The
        // single source call is then sliced to either `effective_extra_sources`
        // (enrichment) or `self.config.fallback_sources` (fallback), preserving
        // the legacy "exactly one source provider call per web_search" contract.
        let speculative_count = effective_extra_sources.max(self.config.fallback_sources);
        let request = self.build_search_request(&input, &[]);

        let grok_future = self.ai.search(&request);
        let speculative_future =
            self.fetch_raw_extra_sources(&input.query, speculative_count, &filters);
        let (grok_result, (raw_sources, raw_origin)) =
            tokio::join!(grok_future, speculative_future);

        let mut response = match grok_result {
            Ok(response) => response,
            Err(err) => {
                return self
                    .finalize_fallback(
                        deadline,
                        session_id,
                        SearchResponse {
                            content: String::new(),
                            sources: Vec::new(),
                        },
                        raw_sources,
                        raw_origin,
                        grok_error_reason(&err),
                        include_content,
                        &filters,
                    )
                    .await;
            }
        };

        let had_grok_sources = !response.sources.is_empty();
        response.sources = filter_sources_by_domains(response.sources, &filters);

        if let Some(reason) = grok_unverifiable_reason(&response) {
            let reason = if reason == "grok_sources_empty" && had_grok_sources {
                "grok_sources_filtered"
            } else {
                reason
            };
            return self
                .finalize_fallback(
                    deadline,
                    session_id,
                    response,
                    raw_sources,
                    raw_origin,
                    reason,
                    include_content,
                    &filters,
                )
                .await;
        }

        let mut enrichment = filter_sources_by_domains(raw_sources, &filters);
        enrichment.truncate(effective_extra_sources);
        let enrichment = with_provider(enrichment, enrichment_label(raw_origin));
        let merged = merge_sources(response.sources, enrichment);
        // SRCH-04 dual gate (zero-regression): skip enrichment when the caller
        // opted out OR there are no supplemental sources. Gating on
        // include_content alone would leave content populated at extra_sources=0
        // and break the legacy "summary + source list" shape.
        let merged = if include_content && effective_extra_sources > 0 {
            crate::enrichment::enrich_sources(
                merged,
                deadline,
                &self.http_client,
                &self.source_router,
                SourceCaps {
                    max_answers: self.config.source_max_answers,
                    max_comments: self.config.source_max_comments,
                },
                self.config.enrich_concurrency,
                self.config.enrich_max_chars,
                self.config.max_inline_sources,
                self.sources.clone(),
                self.fallback_sources.clone(),
            )
            .await
        } else {
            merged
        };

        let merged_arc = Arc::new(merged);
        let sources_count = merged_arc.len();
        self.cache
            .lock()
            .await
            .set(session_id.clone(), merged_arc.clone());

        // The cache keeps the full enriched content; only the returned copy is
        // trimmed to the response budget so drill-down loses nothing.
        let mut out_sources = (*merged_arc).clone();
        let truncated = apply_response_budget(
            response.content.chars().count(),
            &mut out_sources,
            self.config.response_max_chars,
            &session_id,
        );

        Ok(WebSearchOutput {
            session_id,
            content: response.content,
            sources_count,
            sources: out_sources,
            search_provider: "grok_responses".to_string(),
            fallback_used: false,
            fallback_reason: None,
            truncated,
        })
    }

    /// Fetch sources from the primary source provider (or fall through to
    /// firecrawl) without applying a path-specific provider label. The
    /// returned Vec carries each provider's native label ("tavily"/"firecrawl");
    /// the caller re-labels via `with_provider` once the path (enrichment vs
    /// fallback) is known.
    async fn fetch_raw_extra_sources(
        &self,
        query: &str,
        count: usize,
        filters: &SearchFilters,
    ) -> (Vec<Source>, RawSourceOrigin) {
        if count == 0 {
            return (Vec::new(), RawSourceOrigin::None);
        }
        if let Some(provider) = &self.sources {
            if let Ok(sources) = provider.search_sources(query, count, filters).await {
                if !sources.is_empty() {
                    return (sources, RawSourceOrigin::Primary);
                }
            }
        }
        if let Some(provider) = &self.fallback_sources {
            if let Ok(sources) = provider.search_sources(query, count, filters).await {
                if !sources.is_empty() {
                    return (sources, RawSourceOrigin::Fallback);
                }
            }
        }
        (Vec::new(), RawSourceOrigin::None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn finalize_fallback(
        &self,
        deadline: tokio::time::Instant,
        session_id: String,
        response: SearchResponse,
        raw_sources: Vec<Source>,
        raw_origin: RawSourceOrigin,
        reason: &str,
        include_content: bool,
        filters: &SearchFilters,
    ) -> Result<WebSearchOutput> {
        let mut fallback = filter_sources_by_domains(raw_sources, filters);
        fallback.truncate(self.config.fallback_sources);
        let fallback = with_provider(fallback, fallback_label(raw_origin));

        // D-03: the degraded path enriches eagerly �?one-hand evidence is most
        // valuable when there is no verifiable summary, so there is no
        // extra_sources gate here (that gate is the normal web_search path's
        // concern, SRCH-04). The one exception is an explicit include_content=false
        // opt-out, which must be honored everywhere so callers who disabled inline
        // content never pay the extra fetch budget.
        let fallback = if include_content {
            crate::enrichment::enrich_sources(
                fallback,
                deadline,
                &self.http_client,
                &self.source_router,
                SourceCaps {
                    max_answers: self.config.source_max_answers,
                    max_comments: self.config.source_max_comments,
                },
                self.config.enrich_concurrency,
                self.config.enrich_max_chars,
                self.config.max_inline_sources,
                self.sources.clone(),
                self.fallback_sources.clone(),
            )
            .await
        } else {
            fallback
        };

        let fallback_arc = Arc::new(fallback);
        let sources_count = fallback_arc.len();
        self.cache
            .lock()
            .await
            .set(session_id.clone(), fallback_arc.clone());

        let content = if response.content.trim().is_empty() {
            format!(
                "Grok Responses search did not return a verifiable answer. Source fallback returned {sources_count} source(s); evaluate them directly rather than treating any text as a verified answer."
            )
        } else {
            format!(
                "Grok Responses returned an answer without verifiable search sources, so source fallback returned {sources_count} source(s). Original Grok answer was not treated as verified; evaluate the listed sources directly."
            )
        };

        let mut out_sources = (*fallback_arc).clone();
        let truncated = apply_response_budget(
            content.chars().count(),
            &mut out_sources,
            self.config.response_max_chars,
            &session_id,
        );

        Ok(WebSearchOutput {
            session_id,
            content,
            sources_count,
            sources: out_sources,
            search_provider: "source_fallback".to_string(),
            fallback_used: true,
            fallback_reason: Some(reason.to_string()),
            truncated,
        })
    }

    /// Return one page of cached sources for a prior `web_search` session.
    /// `offset`/`limit` follow the official MCP fetch server's `start_index`
    /// continuation pattern, applied to sources; an offset past the end is an
    /// empty page, not an error. Each page is additionally subject to the
    /// response budget (`truncated` reports in-page trimming).
    pub async fn get_sources(
        &self,
        session_id: &str,
        offset: usize,
        limit: Option<usize>,
    ) -> Result<GetSourcesOutput> {
        let cached = self
            .cache
            .lock()
            .await
            .get(session_id)
            .ok_or_else(|| GrokSearchError::NotFound(format!("session_id={session_id}")))?;
        let total_sources = cached.len();
        let start = offset.min(total_sources);
        let end = limit
            .map_or(total_sources, |l| start.saturating_add(l))
            .min(total_sources);
        let mut page: Vec<Source> = cached[start..end].to_vec();
        let truncated =
            apply_response_budget(0, &mut page, self.config.response_max_chars, session_id);
        // Budget trimming may shorten the page; continue from what was
        // actually returned, not from the requested slice end.
        let served_end = start + page.len();
        Ok(GetSourcesOutput {
            session_id: session_id.to_string(),
            sources_count: page.len(),
            sources: page,
            total_sources,
            offset,
            next_offset: (served_end < total_sources).then_some(served_end),
            truncated,
        })
    }

    pub async fn academic_search(
        &self,
        input: AcademicSearchInput,
    ) -> Result<AcademicSearchOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self.academic_service()?.search(input).await;
        self.log_result(&request_id, "academic_search", start, &result, json!({}));
        result
    }

    pub async fn wechat_search(&self, mut input: WechatSearchInput) -> Result<WechatSearchOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let query_chars = input.query.chars().count();
        if input.query.trim().is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "wechat_search.query is required".to_string(),
            ));
        }
        input.max_results = Some(input.max_results.unwrap_or(10));
        input.pages = Some(input.pages.unwrap_or(1));
        input.max_content_chars = input
            .max_content_chars
            .or(self.config.fetch_max_chars)
            .or(Some(self.config.enrich_max_chars));
        let result = self.wechat_provider()?.search(input).await;
        self.log_result(
            &request_id,
            "wechat_search",
            start,
            &result,
            json!({ "query_chars": query_chars }),
        );
        result
    }

    pub async fn zhihu_search(&self, mut input: ZhihuSearchInput) -> Result<ZhihuSearchOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let query_chars = input.query.chars().count();
        if input.query.trim().is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "zhihu_search.query is required".to_string(),
            ));
        }
        input.count = Some(input.count.unwrap_or(10));
        let result = self.zhihu_provider()?.search(input).await;
        self.log_result(
            &request_id,
            "zhihu_search",
            start,
            &result,
            json!({ "query_chars": query_chars }),
        );
        result
    }

    pub async fn academic_get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
        extract_material_links: bool,
    ) -> Result<AcademicGetOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self
            .academic_service()
            .map(|service| {
                service.get(
                    identifier,
                    include_citations,
                    include_open_access,
                    extract_material_links,
                )
            })?
            .await;
        self.log_result(
            &request_id,
            "academic_get",
            start,
            &result,
            json!({ "identifier": identifier }),
        );
        result
    }

    pub async fn academic_citations(
        &self,
        identifier: &str,
        limit: Option<usize>,
    ) -> Result<AcademicCitationsOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self
            .academic_service()?
            .citations(identifier, limit.unwrap_or(10))
            .await;
        self.log_result(
            &request_id,
            "academic_citations",
            start,
            &result,
            json!({ "identifier": identifier, "limit": limit }),
        );
        result
    }

    pub async fn academic_read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicReadOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self
            .academic_service()?
            .read(identifier, url, max_chars, output_format, parse_options)
            .await;
        self.log_result(&request_id, "academic_read", start, &result, json!({}));
        result
    }

    pub async fn academic_parse_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicParsePdfOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self
            .academic_service()?
            .parse_pdf(identifier, url, max_chars, output_format, parse_options)
            .await;
        self.log_result(&request_id, "academic_parse_pdf", start, &result, json!({}));
        result
    }

    pub async fn academic_download_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        output_path: String,
        overwrite: bool,
    ) -> Result<AcademicDownloadPdfOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self
            .academic_service()?
            .download_pdf(identifier, url, output_path, overwrite)
            .await;
        self.log_result(
            &request_id,
            "academic_download_pdf",
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_read(
        &self,
        input: AcademicPdfReadInput,
    ) -> Result<AcademicPdfReadOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self.academic_service()?.pdf_read(input).await;
        self.log_result(&request_id, "academic_pdf_read", start, &result, json!({}));
        result
    }

    pub async fn academic_pdf_structure(
        &self,
        input: AcademicPdfStructureInput,
    ) -> Result<AcademicPdfStructureOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self.academic_service()?.pdf_structure(input).await;
        self.log_result(
            &request_id,
            "academic_pdf_structure",
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_artifacts(
        &self,
        input: AcademicPdfArtifactsInput,
    ) -> Result<AcademicPdfArtifactsOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self.academic_service()?.pdf_artifacts(input).await;
        self.log_result(
            &request_id,
            "academic_pdf_artifacts",
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_download(
        &self,
        input: AcademicPdfDownloadInput,
    ) -> Result<AcademicPdfDownloadOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let result = self.academic_service()?.pdf_download(input).await;
        self.log_result(
            &request_id,
            "academic_pdf_download",
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_progressive_get(
        &self,
        input: AcademicProgressiveGetInput,
    ) -> Result<AcademicProgressiveGetOutput> {
        let request_id = self.logger.request_id();
        let start = Instant::now();
        let cache_key = input.cache_key.clone();
        let result = self.academic_service()?.progressive_get(input).await;
        self.log_result(
            &request_id,
            "academic_progressive_get",
            start,
            &result,
            json!({ "cache_key": cache_key }),
        );
        result
    }

    pub fn warm_academic_institutional_access(&self) {
        if let Some(academic) = &self.academic {
            academic.warm_institutional_access();
        }
    }

    fn academic_service(&self) -> Result<&dyn AcademicServiceProvider> {
        self.academic
            .as_ref()
            .map(|service| service.as_ref())
            .ok_or(GrokSearchError::MissingConfig(
                "GROK_SEARCH_ACADEMIC_ENABLED",
            ))
    }

    fn wechat_provider(&self) -> Result<&dyn WechatProvider> {
        self.wechat
            .as_ref()
            .map(|provider| provider.as_ref())
            .ok_or(GrokSearchError::MissingConfig("wechat provider"))
    }

    fn zhihu_provider(&self) -> Result<&dyn ZhihuProvider> {
        self.zhihu
            .as_ref()
            .map(|provider| provider.as_ref())
            .ok_or(GrokSearchError::MissingConfig(
                "ZHIHU_ACCESS_SECRET or ZHIHU_API_KEY",
            ))
    }

    fn log_result<T>(
        &self,
        request_id: &str,
        operation: &str,
        start: Instant,
        result: &Result<T>,
        payload: serde_json::Value,
    ) {
        match result {
            Ok(_) => self.logger.event(
                request_id,
                "debug",
                &format!("{operation}.success"),
                Some(operation),
                Some(start.elapsed()),
                payload,
            ),
            Err(err) => self.logger.error(
                request_id,
                &format!("{operation}.error"),
                Some(operation),
                Some(start.elapsed()),
                err,
                payload,
            ),
        }
    }

    pub(crate) fn build_search_request(
        &self,
        input: &WebSearchInput,
        extra_sources: &[Source],
    ) -> SearchRequest {
        let mut content = input.query.clone();
        if let Some(platform) = input.platform.as_deref().filter(|value| !value.is_empty()) {
            content.push_str("\n\nFocus platform: ");
            content.push_str(platform);
        }
        if let Some(days) = input.recency_days {
            content.push_str(&format!(
                "\n\nRestrict evidence to sources published within the last {days} day(s)."
            ));
        }
        if !input.include_domains.is_empty() {
            content.push_str("\n\nPrefer sources from: ");
            content.push_str(&input.include_domains.join(", "));
        }
        if !input.exclude_domains.is_empty() {
            content.push_str("\n\nDo not cite sources from: ");
            content.push_str(&input.exclude_domains.join(", "));
        }
        if !extra_sources.is_empty() {
            content.push_str("\n\nAdditional sources:\n");
            for source in extra_sources {
                content.push_str("- ");
                content.push_str(&source.url);
                if let Some(title) = &source.title {
                    content.push_str(" | ");
                    content.push_str(title);
                }
                content.push('\n');
            }
        }

        SearchRequest {
            model: input
                .model
                .clone()
                .unwrap_or_else(|| self.default_model.clone()),
            system: Some("Answer concisely with factual claims grounded in web search sources. Prefer primary sources. If sources are weak or unavailable, say so.".to_string()),
            messages: vec![SearchMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::text(content)],
            }],
            tools: vec![SearchTool::web_search()],
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RawSourceOrigin {
    None,
    Primary,
    Fallback,
}

/// Pick the model the active transport actually understands. Responses speaks
/// Grok-native model names (`grok_model`); the chat-completions gateway speaks
/// whatever `OPENAI_COMPATIBLE_MODEL` declares, falling back to `grok_model`
/// only when the operator hasn't set one. Resolved once at service
/// construction so every outgoing `SearchRequest` carries the right default
/// �?preventing the chat path from silently shipping a Grok-only ID.
fn resolve_default_model(config: &Config) -> String {
    use grok_search_config::Transport;
    match config.transport {
        Transport::Responses => config.grok_model.clone(),
        Transport::ChatCompletions => config
            .openai_compatible_model
            .clone()
            .unwrap_or_else(|| config.grok_model.clone()),
    }
}

fn enrichment_label(origin: RawSourceOrigin) -> &'static str {
    match origin {
        RawSourceOrigin::Primary => "tavily_enrichment",
        RawSourceOrigin::Fallback => "firecrawl_enrichment",
        RawSourceOrigin::None => "tavily_enrichment",
    }
}

fn fallback_label(origin: RawSourceOrigin) -> &'static str {
    match origin {
        RawSourceOrigin::Primary => "tavily_fallback",
        RawSourceOrigin::Fallback => "firecrawl_enrichment",
        RawSourceOrigin::None => "tavily_fallback",
    }
}

/// Maps a failed Grok call to a stable `fallback_reason` identifier. Kept at
/// enum-variant granularity on purpose: distinguishing timeout / auth / parse
/// from a generic provider failure is the diagnostically useful axis, while
/// sub-parsing HTTP status codes out of `Provider(String)` would be fragile.
/// `Provider` (and any other variant) preserves the legacy `grok_provider_error`.
fn grok_error_reason(err: &GrokSearchError) -> &'static str {
    match err {
        GrokSearchError::Timeout(_) => "grok_timeout",
        GrokSearchError::OAuth(_) => "grok_auth_error",
        GrokSearchError::Parse(_) => "grok_parse_error",
        _ => "grok_provider_error",
    }
}

fn grok_unverifiable_reason(response: &SearchResponse) -> Option<&'static str> {
    if response.content.trim().is_empty() {
        return Some("grok_content_empty");
    }
    if response.sources.is_empty() {
        return Some("grok_sources_empty");
    }
    None
}

fn with_provider(
    mut sources: Vec<Source>,
    provider: impl Into<std::borrow::Cow<'static, str>>,
) -> Vec<Source> {
    let provider = provider.into();
    for source in &mut sources {
        source.provider = provider.clone();
    }
    sources
}

pub(crate) struct Probe {
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

impl Probe {
    pub(crate) fn ok(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
        }
    }
    pub(crate) fn failed(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
    pub(crate) fn skipped(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
}

struct FakeAiProvider;

#[async_trait]
impl AiProvider for FakeAiProvider {
    async fn search(&self, _request: &SearchRequest) -> Result<SearchResponse> {
        Ok(SearchResponse {
            content: "OpenAI published a verifiable update.".to_string(),
            sources: vec![
                Source::new("https://openai.com/news", "grok_responses").with_title("OpenAI News")
            ],
        })
    }
}

struct FakeSourceProvider;

#[async_trait]
impl SourceProvider for FakeSourceProvider {
    async fn search_sources(
        &self,
        _query: &str,
        max_results: usize,
        _filters: &SearchFilters,
    ) -> Result<Vec<Source>> {
        Ok((0..max_results)
            .map(|idx| {
                Source::new(format!("https://example.com/source-{idx}"), "tavily")
                    .with_title(format!("Source {idx}"))
            })
            .collect())
    }

    async fn fetch(&self, url: &str) -> Result<String> {
        Ok(format!("Fetched content from {url}"))
    }

    async fn map(&self, url: &str, max_results: usize) -> Result<Vec<Source>> {
        Ok((0..max_results)
            .map(|idx| Source::new(format!("{url}/page-{idx}"), "tavily"))
            .collect())
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
