use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::cache::SourceCache;
use grok_search_config::{AuthMode, Config};
use grok_search_net::proxy::ProxyDiagnostics;
pub use grok_search_provider_core::{AcademicServiceProvider, AiProvider, SourceProvider};
use grok_search_source_core::{
    resolve_content, SourceCaps, SourceRouter, SourceType, NO_SPECIALIST_MATCH,
};
use grok_search_types::model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchResponse, SearchTool,
};
use grok_search_types::model::source::{merge_sources, Source};
use grok_search_types::model::tool::{
    GetSourcesOutput, WebFetchOutput, WebSearchInput, WebSearchOutput,
};
use grok_search_types::{
    AcademicCitationsOutput, AcademicGetOutput, AcademicReadOutput, AcademicSearchInput,
    AcademicSearchOutput,
};
use grok_search_types::{GrokSearchError, Result};

#[derive(Clone)]
pub struct SearchService {
    config: Config,
    ai: Arc<dyn AiProvider>,
    /// Model name written into every `SearchRequest` produced by the service.
    /// Resolved once from `config` at construction so each transport gets the
    /// model it actually understands: `grok_model` for Responses, and
    /// `openai_compatible_model` (falling back to `grok_model`) for the
    /// chat-completions transport. Per-call overrides via `WebSearchInput.model`
    /// still win.
    default_model: String,
    sources: Option<Arc<dyn SourceProvider>>,
    fallback_sources: Option<Arc<dyn SourceProvider>>,
    cache: Arc<Mutex<SourceCache>>,
    /// Shared reqwest client for the sources pipeline (same instance handed to
    /// providers). Stored here because resolve_content needs direct GET access.
    http_client: reqwest::Client,
    /// Specialist extractor router. Empty in Phase 1. Behind `Arc` so
    /// `SearchService: Clone` still holds (the router is not `Clone`).
    source_router: Arc<SourceRouter>,
    proxy_diagnostics: ProxyDiagnostics,
    academic: Option<Arc<dyn AcademicServiceProvider>>,
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
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30)),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
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
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30)),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
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
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30)),
            source_router: Arc::new(router),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
        }
    }

    pub async fn web_search(&self, input: WebSearchInput) -> Result<WebSearchOutput> {
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
            enrich_sources(
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
            enrich_sources(
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

    pub async fn web_fetch(&self, url: &str, max_chars: Option<usize>) -> Result<WebFetchOutput> {
        validate_http_url(url)?;
        let effective_limit = max_chars.or(self.config.fetch_max_chars);

        let (content, source_type, fallback_reason) = match url::Url::parse(url) {
            Ok(parsed) => {
                match resolve_content(
                    &self.http_client,
                    &parsed,
                    self.source_router.as_ref(),
                    &SourceCaps {
                        max_answers: self.config.source_max_answers,
                        max_comments: self.config.source_max_comments,
                    },
                )
                .await
                {
                    // Specialist succeeded �?keep its content and source type.
                    Ok((content, kind)) => (content, kind, None),
                    // No specialist matched: go generic silently (D-01).
                    Err(reason) if reason == NO_SPECIALIST_MATCH => {
                        let generic = self.web_fetch_raw(url).await?;
                        (generic, SourceType::Generic, None)
                    }
                    // Specialist matched but failed/empty: surface the reason (D-01).
                    Err(reason) => {
                        let generic = self.web_fetch_raw(url).await?;
                        (generic, SourceType::Generic, Some(reason))
                    }
                }
            }
            // Malformed URL is not a specialist failure �?go generic, no reason.
            Err(_) => {
                let generic = self.web_fetch_raw(url).await?;
                (generic, SourceType::Generic, None)
            }
        };

        Ok(apply_fetch_limit(
            url,
            content,
            effective_limit,
            source_type,
            fallback_reason,
        ))
    }

    async fn web_fetch_raw(&self, url: &str) -> Result<String> {
        generic_source_fetch(&self.sources, &self.fallback_sources, url).await
    }

    pub async fn web_map(&self, url: &str, max_results: usize) -> Result<Vec<Source>> {
        validate_http_url(url)?;
        self.sources
            .as_ref()
            .ok_or(GrokSearchError::MissingConfig("TAVILY_API_KEY"))?
            .map(url, max_results)
            .await
    }

    pub async fn academic_search(
        &self,
        input: AcademicSearchInput,
    ) -> Result<AcademicSearchOutput> {
        self.academic_service()?.search(input).await
    }

    pub async fn academic_get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
    ) -> Result<AcademicGetOutput> {
        self.academic_service()
            .map(|service| service.get(identifier, include_citations, include_open_access))?
            .await
    }

    pub async fn academic_citations(
        &self,
        identifier: &str,
        limit: Option<usize>,
    ) -> Result<AcademicCitationsOutput> {
        self.academic_service()?
            .citations(identifier, limit.unwrap_or(10))
            .await
    }

    pub async fn academic_read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
    ) -> Result<AcademicReadOutput> {
        self.academic_service()?
            .read(identifier, url, max_chars, output_format)
            .await
    }

    fn academic_service(&self) -> Result<&dyn AcademicServiceProvider> {
        self.academic
            .as_ref()
            .map(|service| service.as_ref())
            .ok_or(GrokSearchError::MissingConfig(
                "GROK_SEARCH_ACADEMIC_ENABLED",
            ))
    }

    /// Runtime diagnostics with live connectivity probes against each configured backend.
    /// Returns provider availability flags, masked config, and per-provider reachability.
    pub async fn doctor(&self) -> serde_json::Value {
        use grok_search_config::Transport;
        let grok_probe = self.probe_grok().await;
        let tavily_probe = match &self.sources {
            Some(provider) => probe_source(provider.as_ref(), "https://example.com").await,
            None => Probe::skipped("TAVILY_API_KEY not configured"),
        };
        let firecrawl_probe = match &self.fallback_sources {
            Some(provider) => probe_source(provider.as_ref(), "https://example.com").await,
            None => Probe::skipped("FIRECRAWL_API_KEY not configured"),
        };

        // Surface the AI transport that the service actually dispatches to so
        // doctor() stays truthful when callers point us at an OpenAI-compatible
        // gateway. The legacy "grok" node name is preserved for backward
        // compatibility, but its fields are now sourced from `default_model`
        // and the transport-appropriate API URL �?never silently from
        // `grok_model` / `grok_api_url` on the chat-completions path.
        let (provider_label, ai_api_url, ai_x_search_enabled) = match self.config.transport {
            Transport::Responses => (
                "grok_responses",
                self.config.grok_api_url.as_str(),
                self.config.x_search_enabled,
            ),
            Transport::ChatCompletions => (
                "openai_compatible",
                self.config
                    .openai_compatible_api_url
                    .as_deref()
                    .unwrap_or(""),
                // x_search is silently ignored on the chat-completions transport
                // (the gateway has no equivalent); report it as disabled rather
                // than leaking a misleading config flag.
                false,
            ),
        };

        serde_json::json!({
            "provider": provider_label,
            "transport": provider_label,
            "grok": {
                "api_url": ai_api_url,
                "model": self.default_model,
                "auth_mode": match self.config.grok_auth_mode {
                    AuthMode::ApiKey => "api_key",
                    AuthMode::OAuth => "oauth",
                },
                "auth_file": self.config
                    .grok_auth_file
                    .clone()
                    .or_else(grok_search_config::auth_path)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "unavailable".to_string()),
                "web_search_enabled": self.config.web_search_enabled,
                "x_search_enabled": ai_x_search_enabled,
                "reachable": grok_probe.ok,
                "detail": grok_probe.detail,
            },
            "tavily": {
                "api_url": self.config.tavily_api_url,
                "enabled": self.config.tavily_enabled,
                "reachable": tavily_probe.ok,
                "detail": tavily_probe.detail,
            },
            "firecrawl": {
                "api_url": self.config.firecrawl_api_url,
                "enabled": self.config.firecrawl_enabled,
                "reachable": firecrawl_probe.ok,
                "detail": firecrawl_probe.detail,
            },
            "default_extra_sources": self.config.default_extra_sources,
            "fallback_sources": self.config.fallback_sources,
            "cache_size": self.config.cache_size,
            "timeout_seconds": self.config.timeout.as_secs(),
            "github_token": self.config.github_token_status(),
            "proxy": self.proxy_diagnostics.to_json(),
            "academic": self
                .academic
                .as_ref()
                .map(|academic| academic.diagnostics())
                .unwrap_or_else(|| serde_json::json!({ "enabled": false })),
            "redacted": self.config.redacted_diagnostics()
        })
    }

    async fn probe_grok(&self) -> Probe {
        // Mirror the real search shape so the probe doesn't fail the
        // adapter's "web_search tool intent" pre-check.
        let mut tools = Vec::new();
        if self.config.web_search_enabled {
            tools.push(SearchTool::web_search());
        }
        let request = SearchRequest {
            model: self.default_model.clone(),
            system: None,
            messages: vec![SearchMessage {
                role: "user".to_string(),
                content: vec![ContentBlock::text("ping")],
            }],
            tools,
        };
        match self.ai.search(&request).await {
            Ok(_) => Probe::ok("grok responded"),
            Err(err) => Probe::failed(err.to_string()),
        }
    }

    fn build_search_request(
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

fn filter_sources_by_domains(sources: Vec<Source>, filters: &SearchFilters) -> Vec<Source> {
    let include = DomainSet::new(&filters.include_domains);
    let exclude = DomainSet::new(&filters.exclude_domains);
    if include.is_empty() && exclude.is_empty() {
        return sources;
    }
    sources
        .into_iter()
        .filter(|source| source_allowed_by_domains(&source.url, &include, &exclude))
        .collect()
}

fn source_allowed_by_domains(url: &str, include: &DomainSet, exclude: &DomainSet) -> bool {
    let Some(host) = host_for_match(url) else {
        return include.is_empty();
    };
    (include.is_empty() || include.matches(&host)) && !exclude.matches(&host)
}

#[derive(Debug, Clone)]
struct DomainSet {
    domains: Vec<String>,
}

impl DomainSet {
    fn new(raw: &[String]) -> Self {
        let domains = raw
            .iter()
            .filter_map(|value| normalize_domain(value))
            .collect();
        Self { domains }
    }

    fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    fn matches(&self, host: &str) -> bool {
        self.domains
            .iter()
            .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
    }
}

fn host_for_match(raw: &str) -> Option<String> {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(normalize_host))
        .or_else(|| normalize_domain(raw))
}

fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(trimmed) {
        return url.host_str().map(normalize_host);
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    let host = normalize_host(host);
    (!host.is_empty()).then_some(host)
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}

fn validate_http_url(raw: &str) -> Result<url::Url> {
    let parsed = url::Url::parse(raw).map_err(|_| {
        GrokSearchError::InvalidParams(
            "url must be an absolute http or https URL with a host".to_string(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(GrokSearchError::InvalidParams(
            "url must be an absolute http or https URL with a host".to_string(),
        ));
    }
    Ok(parsed)
}

fn apply_fetch_limit(
    url: &str,
    mut content: String,
    max_chars: Option<usize>,
    source_type: SourceType,
    fallback_reason: Option<String>,
) -> WebFetchOutput {
    let Some(limit) = max_chars else {
        let original_length = content.chars().count();
        return WebFetchOutput {
            url: url.to_string(),
            content,
            original_length,
            truncated: false,
            source_type,
            fallback_reason,
        };
    };

    let mut count = 0usize;
    let mut cutoff: Option<usize> = None;
    for (byte_idx, _) in content.char_indices() {
        if count == limit {
            cutoff = Some(byte_idx);
            break;
        }
        count += 1;
    }

    match cutoff {
        Some(byte_idx) => {
            let extra = content[byte_idx..].chars().count();
            content.truncate(byte_idx);
            WebFetchOutput {
                url: url.to_string(),
                content,
                original_length: limit + extra,
                truncated: true,
                source_type,
                fallback_reason,
            }
        }
        None => WebFetchOutput {
            url: url.to_string(),
            content,
            original_length: count,
            truncated: false,
            source_type,
            fallback_reason,
        },
    }
}

/// Generic (non-specialist) content fetch via the configured source providers:
/// primary (Tavily) first, then fallback (Firecrawl). Shared by `web_fetch` and
/// inline enrichment so both agree on how an ordinary URL is retrieved once no
/// specialist extractor matches. Returns `MissingConfig` when neither provider
/// is configured.
async fn generic_source_fetch(
    primary: &Option<Arc<dyn SourceProvider>>,
    fallback: &Option<Arc<dyn SourceProvider>>,
    url: &str,
) -> Result<String> {
    if let Some(provider) = primary {
        if let Ok(content) = provider.fetch(url).await {
            if !content.trim().is_empty() {
                return Ok(content);
            }
        }
    }
    if let Some(provider) = fallback {
        return provider.fetch(url).await;
    }
    Err(GrokSearchError::MissingConfig(
        "TAVILY_API_KEY or FIRECRAWL_API_KEY",
    ))
}

/// Concurrently back-fill `Source.content` for the first `max_sources` sources
/// via the Phase 1 `resolve_content` pipeline; later sources stay
/// metadata-only (content = None) so a Grok response with dozens of citations
/// cannot blow up the payload �?agents drill into them with `web_fetch`.
/// Bounded by `concurrency` (Semaphore) and the shared `deadline` (D-02:
/// per-source `timeout_at`, not an independent budget). Every enriched source
/// ends with `content = Some(..)` �?real markdown (truncated to `max_chars`)
/// on success, or a deterministic `_Failed to retrieve: ..._` note on any
/// failure/timeout/invalid-url (D-05 within the inline window: never None,
/// never empty). Source order is preserved.
#[allow(clippy::too_many_arguments)]
async fn enrich_sources(
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
            // acquire is micro-second scale for concurrency<=5; deadline
            // enforcement applies to the resolve_content call itself.
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
                        // Specialist produced no content �?either no specialist
                        // matched (generic URL) OR a matched specialist's API
                        // failed/rate-limited/rendered empty. Either way, mirror
                        // web_fetch and try the configured Tavily/Firecrawl generic
                        // fetch before giving up, so inline content still has page
                        // evidence when a source provider can fetch the URL (P1 +
                        // specialist-failure fallback). The original `reason` is
                        // surfaced only if the generic fetch also fails.
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

/// Approximate serialized footprint of one source: every metadata field plus
/// inline content plus a fixed allowance for JSON keys/quotes/separators. The
/// budget must track what actually lands in the agent's context �?a broad
/// query where Grok cites 50+ pages overflows on metadata alone, so counting
/// only inline content under-reports the payload.
fn source_weight(source: &Source) -> usize {
    const JSON_OVERHEAD: usize = 64;
    let opt_chars = |v: &Option<String>| v.as_deref().map(|s| s.chars().count()).unwrap_or(0);
    source.url.chars().count()
        + source.provider.chars().count()
        + opt_chars(&source.title)
        + opt_chars(&source.description)
        + opt_chars(&source.published_date)
        + source
            .content
            .as_deref()
            .map(|c| c.chars().count())
            .unwrap_or(0)
        + JSON_OVERHEAD
}

/// Trim the response from the TAIL until `answer_chars` plus the weighted
/// source list fits the `budget`. Head sources (Grok's own citations rank
/// first) survive intact. Two passes:
///
/// 1. Replace tail inline content with an actionable note naming `web_fetch`
///    and `get_sources` �?the official MCP fetch server's "call again with
///    start_index" guidance, applied to sources.
/// 2. Still over budget (metadata overflow): drop whole tail sources from the
///    returned list, always keeping at least one.
///
/// The synthesized answer is never trimmed. Returns whether anything was
/// trimmed; callers always trim a clone so the session cache keeps everything.
fn apply_response_budget(
    answer_chars: usize,
    sources: &mut Vec<Source>,
    budget: usize,
    session_id: &str,
) -> bool {
    let content_chars = |s: &Source| s.content.as_deref().map(|c| c.chars().count()).unwrap_or(0);
    let mut total: usize = answer_chars + sources.iter().map(source_weight).sum::<usize>();
    if total <= budget {
        return false;
    }

    // Pass 1: swap tail inline content for recovery notes.
    for idx in (0..sources.len()).rev() {
        if total <= budget {
            break;
        }
        let len = content_chars(&sources[idx]);
        if len == 0 {
            continue;
        }
        let url = sources[idx].url.clone();
        let note = |verb: &str| {
            format!(
                "_[{verb}: response budget reached �?full text via web_fetch(\"{url}\") or get_sources(session_id=\"{session_id}\", offset={idx}, limit=1)]_"
            )
        };
        let omit_note = note("inline content omitted");
        let omit_len = omit_note.chars().count();
        if len <= omit_len {
            // Replacing would not shrink the payload; leave it alone.
            continue;
        }
        let overshoot = total - budget;
        let trim_note = note("truncated");
        // "\n\n" separator + note must fit inside the chars we reclaim.
        let trim_overhead = trim_note.chars().count() + 2;
        if len > overshoot + trim_overhead {
            // Partial trim: keep a prefix so the head of the document survives.
            let keep = len - overshoot - trim_overhead;
            let prefix: String = sources[idx]
                .content
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(keep)
                .collect();
            sources[idx].content = Some(format!("{prefix}\n\n{trim_note}"));
            total -= overshoot;
        } else {
            sources[idx].content = Some(omit_note);
            total = total - len + omit_len;
        }
    }

    // Pass 2: metadata alone still over budget �?cut whole tail sources.
    // They stay in the cache; get_sources(offset=..) pages through them.
    while total > budget && sources.len() > 1 {
        let dropped = sources.pop().expect("len > 1");
        total -= source_weight(&dropped);
    }

    true
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

struct Probe {
    ok: bool,
    detail: String,
}

impl Probe {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
        }
    }
    fn failed(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
    fn skipped(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
        }
    }
}

async fn probe_source(provider: &dyn SourceProvider, sample_url: &str) -> Probe {
    // Use a short keyword search as a lightweight liveness signal.
    let filters = SearchFilters::default();
    match provider.search_sources("ping", 1, &filters).await {
        Ok(_) => Probe::ok(format!("reachable (sample probe via {sample_url} ok)")),
        Err(err) => Probe::failed(err.to_string()),
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
