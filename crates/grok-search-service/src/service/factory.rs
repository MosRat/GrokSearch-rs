use async_trait::async_trait;

use super::*;

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
}

pub(crate) fn resolve_default_model(config: &Config) -> String {
    use grok_search_config::Transport;
    match config.transport {
        Transport::Responses => config.grok_model.clone(),
        Transport::ChatCompletions => config
            .openai_compatible_model
            .clone()
            .unwrap_or_else(|| config.grok_model.clone()),
    }
}

pub(crate) struct FakeAiProvider;

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

pub(crate) struct FakeSourceProvider;

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
