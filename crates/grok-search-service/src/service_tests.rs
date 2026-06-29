use super::*;
use grok_search_source_core::SourceType;

#[cfg(test)]
mod transport_dispatch_tests {
    use super::*;
    use grok_search_config::Transport;

    #[test]
    fn default_model_follows_chat_completions_when_compat_model_set() {
        // Reproduces the regression: SearchService::build_search_request used
        // to stamp `grok_model` into every SearchRequest, masking
        // OPENAI_COMPATIBLE_MODEL on the chat-completions transport.
        let config = Config::from_env_map([
            ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
            ("OPENAI_COMPATIBLE_MODEL", "gpt-4o-mini"),
            ("GROK_SEARCH_MODEL", "grok-4-1-fast-reasoning"),
        ]);
        assert_eq!(config.transport, Transport::ChatCompletions);
        assert_eq!(resolve_default_model(&config), "gpt-4o-mini");
    }

    #[test]
    fn default_model_falls_back_to_grok_model_when_compat_model_missing() {
        let config = Config::from_env_map([
            ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
            ("GROK_SEARCH_MODEL", "grok-4-1-fast-reasoning"),
        ]);
        assert_eq!(config.transport, Transport::ChatCompletions);
        assert_eq!(resolve_default_model(&config), "grok-4-1-fast-reasoning");
    }

    #[test]
    fn default_model_uses_grok_model_on_responses_transport() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GROK_SEARCH_MODEL", "grok-4-1-fast-reasoning"),
            ("OPENAI_COMPATIBLE_MODEL", "gpt-4o-mini"),
        ]);
        assert_eq!(config.transport, Transport::Responses);
        assert_eq!(resolve_default_model(&config), "grok-4-1-fast-reasoning");
    }

    #[tokio::test]
    async fn doctor_reports_openai_compatible_transport_fields() {
        // Regression: doctor() used to hardcode "grok_responses" / grok_model /
        // grok_api_url, masking what the service actually dispatches to on the
        // chat-completions transport. Now it must reflect compat config.
        let config = Config::from_env_map([
            ("OPENAI_COMPATIBLE_API_URL", "https://compat.example/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
            ("OPENAI_COMPATIBLE_MODEL", "gpt-4o-mini"),
            ("GROK_SEARCH_MODEL", "grok-4-1-fast-reasoning"),
            // X-search is silently ignored on this transport �?doctor must
            // report the effective behavior (false), not the raw env flag.
            ("GROK_SEARCH_X_SEARCH", "true"),
        ]);
        assert_eq!(config.transport, Transport::ChatCompletions);

        // Hand-build the service with fake AI to avoid any real HTTP from
        // probe_grok during doctor().
        let svc = SearchService {
            default_model: resolve_default_model(&config),
            config,
            ai: Arc::new(FakeAiProvider),
            sources: None,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(16))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        };

        let report = svc.doctor().await;
        assert_eq!(report["provider"], "openai_compatible");
        assert_eq!(report["transport"], "openai_compatible");
        assert_eq!(report["grok"]["api_url"], "https://compat.example/v1");
        assert_eq!(report["grok"]["model"], "gpt-4o-mini");
        assert_eq!(report["grok"]["x_search_enabled"], false);
    }

    #[tokio::test]
    async fn doctor_still_reports_grok_responses_on_responses_transport() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GROK_SEARCH_MODEL", "grok-4-1-fast-reasoning"),
        ]);
        assert_eq!(config.transport, Transport::Responses);

        let svc = SearchService {
            default_model: resolve_default_model(&config),
            config,
            ai: Arc::new(FakeAiProvider),
            sources: None,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(16))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        };

        let report = svc.doctor().await;
        assert_eq!(report["provider"], "grok_responses");
        assert_eq!(report["grok"]["model"], "grok-4-1-fast-reasoning");
    }

    #[tokio::test]
    async fn doctor_reports_github_token_status() {
        // With GITHUB_TOKEN set -> "set", and the raw value never leaks.
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GITHUB_TOKEN", "ghp_test"),
        ]);
        let svc = SearchService {
            default_model: resolve_default_model(&config),
            config,
            ai: Arc::new(FakeAiProvider),
            sources: None,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(16))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        };
        let report = svc.doctor().await;
        assert_eq!(report["github_token"], "set");
        // No-leak: the full report must not contain the token value anywhere.
        assert!(
            !report.to_string().contains("ghp_test"),
            "token value leaked into doctor report: {report}"
        );

        // Without GITHUB_TOKEN -> "unset".
        let config_unset = Config::from_env_map([("GROK_SEARCH_API_KEY", "xai-fake")]);
        let svc_unset = SearchService {
            default_model: resolve_default_model(&config_unset),
            config: config_unset,
            ai: Arc::new(FakeAiProvider),
            sources: None,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(16))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        };
        let report_unset = svc_unset.doctor().await;
        assert_eq!(report_unset["github_token"], "unset");
    }

    #[tokio::test]
    async fn doctor_reports_proxy_diagnostics_without_credentials() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GROK_SEARCH_PROXY", "http://user:secret@127.0.0.1:7890"),
        ]);
        let svc = SearchService {
            default_model: resolve_default_model(&config),
            config,
            ai: Arc::new(FakeAiProvider),
            sources: None,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(16))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(SourceRouter::default()),
            proxy_diagnostics: ProxyDiagnostics {
                mode: "manual".to_string(),
                status: "proxied".to_string(),
                source: "manual".to_string(),
                url_redacted: Some(grok_search_net::proxy::redact_proxy_url(
                    "http://user:secret@127.0.0.1:7890",
                )),
                detail: "ok".to_string(),
                checked_urls: vec!["https://api.x.ai/v1".to_string()],
            },
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        };

        let report = svc.doctor().await;
        assert_eq!(report["proxy"]["status"], "proxied");
        let rendered = report.to_string();
        assert!(rendered.contains("***"));
        assert!(!rendered.contains("user:secret"));
        assert!(!rendered.contains("secret@"));
    }

    #[tokio::test]
    async fn fake_with_router_constructs_and_clones() {
        let svc = SearchService::fake_with_router(
            Arc::new(FakeSourceProvider),
            None,
            SourceRouter::default(),
        );
        // SearchService derives Clone; storing Arc<SourceRouter> must preserve it.
        let _clone = svc.clone();
    }
}

#[cfg(test)]
mod enrich_tests {
    use super::*;
    use grok_search_source_core::{SourceExtractor, SourceRouter};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use url::Url;

    /// Always-matching extractor that records peak concurrency and returns a
    /// fixed body after a visibility sleep.
    struct CountingExtractor {
        peak: Arc<AtomicUsize>,
        current: Arc<AtomicUsize>,
        sleep_ms: u64,
    }
    #[async_trait]
    impl SourceExtractor for CountingExtractor {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::Wikipedia
        }
        async fn fetch_render(
            &self,
            _c: &reqwest::Client,
            _u: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            let n = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(n, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            Ok("content".to_string())
        }
    }

    /// URL-discriminating failure extractor: matches ONLY urls containing
    /// `fail_url_marker`, so a router can route one source here and the rest to
    /// CountingExtractor (true fault isolation).
    struct MarkerErrExtractor {
        fail_url_marker: String,
    }
    #[async_trait]
    impl SourceExtractor for MarkerErrExtractor {
        fn matches(&self, url: &Url) -> bool {
            url.as_str().contains(&self.fail_url_marker)
        }
        fn kind(&self) -> SourceType {
            SourceType::GithubIssue
        }
        async fn fetch_render(
            &self,
            _c: &reqwest::Client,
            _u: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            Err(grok_search_types::GrokSearchError::Provider(
                "always_fails".to_string(),
            ))
        }
    }

    /// Returns an oversized body to exercise the per-source char cap.
    struct OversizeExtractor {
        len: usize,
    }
    #[async_trait]
    impl SourceExtractor for OversizeExtractor {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::Wikipedia
        }
        async fn fetch_render(
            &self,
            _c: &reqwest::Client,
            _u: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            Ok("x".repeat(self.len))
        }
    }

    /// Hangs far past any test deadline �?used to trigger the timeout note.
    struct HangingExtractor;
    #[async_trait]
    impl SourceExtractor for HangingExtractor {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::Wikipedia
        }
        async fn fetch_render(
            &self,
            _c: &reqwest::Client,
            _u: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Ok("never".to_string())
        }
    }

    /// Supplemental provider whose `search_sources` returns example.com sources
    /// but whose generic `fetch` always errors �?used to exercise the
    /// "specialist failed AND generic fetch failed �?note" path.
    struct SearchOkFetchErrProvider;
    #[async_trait]
    impl SourceProvider for SearchOkFetchErrProvider {
        async fn search_sources(
            &self,
            _query: &str,
            max_results: usize,
            _filters: &SearchFilters,
        ) -> Result<Vec<Source>> {
            Ok((0..max_results)
                .map(|idx| Source::new(format!("https://example.com/source-{idx}"), "tavily"))
                .collect())
        }
        async fn fetch(&self, _url: &str) -> Result<String> {
            Err(grok_search_types::GrokSearchError::Provider(
                "generic fetch unavailable".to_string(),
            ))
        }
        async fn map(&self, _url: &str, _max_results: usize) -> Result<Vec<Source>> {
            Ok(Vec::new())
        }
    }

    /// Build a SearchService with fake AI + a caller-supplied supplemental
    /// provider, router, and config. Mirrors the doctor_* struct-literal tests.
    fn service_with_sources(
        config: Config,
        router: SourceRouter,
        sources: Option<Arc<dyn SourceProvider>>,
    ) -> SearchService {
        SearchService {
            default_model: resolve_default_model(&config),
            config,
            ai: Arc::new(FakeAiProvider),
            sources,
            fallback_sources: None,
            cache: Arc::new(Mutex::new(SourceCache::new(64))),
            http_client: grok_search_net::http::build_client(std::time::Duration::from_secs(30))
                .expect("test HTTP client"),
            source_router: Arc::new(router),
            proxy_diagnostics: ProxyDiagnostics::default(),
            academic: None,
            wechat: None,
            zhihu: None,
            logger: DebugLogger::disabled(),
        }
    }

    fn service_with(config: Config, router: SourceRouter) -> SearchService {
        service_with_sources(config, router, Some(Arc::new(FakeSourceProvider)))
    }

    fn enrich_config() -> Config {
        Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "fake-grok"),
            ("TAVILY_API_KEY", "fake-tavily"),
        ])
    }

    fn base_input() -> WebSearchInput {
        WebSearchInput {
            query: "q".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn counting_extractor_self_test() {
        // Sanity: the helper itself records concurrency.
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak: Arc::clone(&peak),
            current: Arc::clone(&current),
            sleep_ms: 5,
        })]);
        let svc = service_with(enrich_config(), router);
        let _ = svc.web_search(base_input()).await.expect("web_search");
        assert!(peak.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn web_search_inline_default_fills_content() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak,
            current,
            sleep_ms: 0,
        })]);
        let svc = service_with(enrich_config(), router);
        let out = svc.web_search(base_input()).await.expect("web_search");

        assert!(!out.sources.is_empty());
        for s in &out.sources {
            let c = s.content.as_deref().unwrap_or("");
            assert!(!c.is_empty(), "every source must have non-empty content");
        }
    }

    #[tokio::test]
    async fn enrich_generic_url_uses_provider_fetch_fallback() {
        // No specialist matches the supplemental URLs �?inline enrichment must
        // fall back to the configured source provider's generic fetch (mirroring
        // web_fetch), not emit a `_Failed to retrieve: no_specialist_match_`
        // note for ordinary search results (P1).
        let svc = service_with(enrich_config(), SourceRouter::default());
        let out = svc.web_search(base_input()).await.expect("web_search");

        assert!(!out.sources.is_empty());
        for s in &out.sources {
            let c = s.content.as_deref().unwrap_or("");
            assert!(
                c.starts_with("Fetched content from"),
                "generic source must use the provider fetch fallback, got: {c:?}"
            );
            assert!(
                !c.contains("no_specialist_match"),
                "must not leak the no_specialist_match note: {c:?}"
            );
        }
    }

    #[tokio::test]
    async fn enrich_concurrency_is_bounded() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak: Arc::clone(&peak),
            current: Arc::clone(&current),
            sleep_ms: 25, // wide enough window for overlap to register
        })]);
        let mut config = enrich_config();
        config.enrich_concurrency = 2;
        let svc = service_with(config, router);

        let _ = svc.web_search(base_input()).await.expect("web_search");
        // 4 sources, concurrency 2 �?peak must never exceed 2.
        assert!(
            peak.load(Ordering::SeqCst) <= 2,
            "peak={}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn enrich_truncates_to_max_chars() {
        let router =
            SourceRouter::with_extractors(vec![Box::new(OversizeExtractor { len: 20_000 })]);
        let svc = service_with(enrich_config(), router); // default enrich_max_chars = 15000
        let out = svc.web_search(base_input()).await.expect("web_search");

        for s in &out.sources {
            let len = s.content.as_deref().map(|c| c.chars().count()).unwrap_or(0);
            assert!(len <= 15_000, "content len {len} exceeds cap");
            assert!(len > 0);
        }
    }

    #[tokio::test]
    async fn enrich_fault_isolation_one_fails_rest_ok() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![
            Box::new(MarkerErrExtractor {
                fail_url_marker: "openai.com".to_string(),
            }),
            Box::new(CountingExtractor {
                peak,
                current,
                sleep_ms: 0,
            }),
        ]);
        // Provider whose generic fetch ALSO fails, so the failing specialist
        // source genuinely falls through to the note (not the generic rescue).
        let svc = service_with_sources(
            enrich_config(),
            router,
            Some(Arc::new(SearchOkFetchErrProvider)),
        );
        let out = svc
            .web_search(base_input())
            .await
            .expect("web_search returns Ok despite one failure");

        let failed = out
            .sources
            .iter()
            .find(|s| s.url.contains("openai.com"))
            .expect("grok source present");
        let passed = out
            .sources
            .iter()
            .find(|s| s.url.contains("example.com"))
            .expect("supplemental source present");

        assert!(
            failed
                .content
                .as_deref()
                .unwrap_or("")
                .starts_with("_Failed to retrieve:"),
            "failing source must carry a failure note, got: {:?}",
            failed.content
        );
        let pc = passed.content.as_deref().unwrap_or("");
        assert!(
            !pc.is_empty() && !pc.starts_with("_Failed to retrieve:"),
            "passing source must carry real content, got: {pc:?}"
        );
    }

    #[tokio::test]
    async fn enrich_specialist_failure_rescued_by_generic_fetch() {
        // A matched specialist whose API errors must fall back to the configured
        // generic fetch (mirroring web_fetch), not store a failure note, when a
        // source provider can still fetch the URL.
        let router = SourceRouter::with_extractors(vec![Box::new(MarkerErrExtractor {
            fail_url_marker: "openai.com".to_string(),
        })]);
        let svc = service_with(enrich_config(), router); // FakeSourceProvider.fetch succeeds
        let out = svc.web_search(base_input()).await.expect("web_search");

        let failed = out
            .sources
            .iter()
            .find(|s| s.url.contains("openai.com"))
            .expect("grok source present");
        let content = failed.content.as_deref().unwrap_or("");
        assert!(
            content.starts_with("Fetched content from"),
            "specialist failure must be rescued by generic fetch, got: {content:?}"
        );
        assert!(
            !content.starts_with("_Failed to retrieve:"),
            "must not store a failure note when generic fetch succeeds: {content:?}"
        );
    }

    #[tokio::test]
    async fn enrich_timeout_yields_note_not_error() {
        let router = SourceRouter::with_extractors(vec![Box::new(HangingExtractor)]);
        let mut config = enrich_config();
        config.timeout = Duration::from_millis(50); // deadline fires fast
        let svc = service_with(config, router);

        let out = svc
            .web_search(base_input())
            .await
            .expect("web_search returns Ok on timeout");
        for s in &out.sources {
            assert!(
                s.content.as_deref().unwrap_or("").contains("timeout"),
                "expected timeout note, got: {:?}",
                s.content
            );
        }
    }

    #[tokio::test]
    async fn include_content_false_omits_content_field() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak,
            current,
            sleep_ms: 0,
        })]);
        let svc = service_with(enrich_config(), router);

        let mut input = base_input();
        input.include_content = Some(false);
        let out = svc.web_search(input).await.expect("web_search");

        for s in &out.sources {
            assert!(s.content.is_none());
            let value = serde_json::to_value(s).unwrap();
            assert!(
                value.get("content").is_none(),
                "JSON must omit the content key, not emit null"
            );
        }
    }

    #[tokio::test]
    async fn extra_sources_zero_suppresses_inline() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak,
            current,
            sleep_ms: 0,
        })]);
        let svc = service_with(enrich_config(), router);

        let mut input = base_input();
        input.extra_sources = Some(0); // effective_extra_sources == 0 �?dual gate suppresses enrich
        let out = svc.web_search(input).await.expect("web_search");

        for s in &out.sources {
            assert!(
                s.content.is_none(),
                "extra_sources=0 must keep the legacy no-content shape"
            );
        }
    }

    #[tokio::test]
    async fn get_sources_inherits_enriched_content() {
        let peak = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let router = SourceRouter::with_extractors(vec![Box::new(CountingExtractor {
            peak,
            current,
            sleep_ms: 0,
        })]);
        let svc = service_with(enrich_config(), router);

        let out = svc.web_search(base_input()).await.expect("web_search");
        let again = svc
            .get_sources(&out.session_id, 0, None)
            .await
            .expect("get_sources");

        assert_eq!(out.sources.len(), again.sources.len());
        for (a, b) in out.sources.iter().zip(again.sources.iter()) {
            assert_eq!(a.url, b.url);
            assert_eq!(
                a.content, b.content,
                "get_sources must reuse the cached enriched content"
            );
        }
    }
}

#[cfg(test)]
mod domain_filter_tests {
    use super::*;

    struct MixedDomainAiProvider {
        sources: Vec<Source>,
    }

    #[async_trait]
    impl AiProvider for MixedDomainAiProvider {
        async fn search(&self, _request: &SearchRequest) -> Result<SearchResponse> {
            Ok(SearchResponse {
                content: "verified answer".to_string(),
                sources: self.sources.clone(),
            })
        }
    }

    struct MixedDomainSourceProvider;

    #[async_trait]
    impl SourceProvider for MixedDomainSourceProvider {
        async fn search_sources(
            &self,
            _query: &str,
            _max_results: usize,
            _filters: &SearchFilters,
        ) -> Result<Vec<Source>> {
            Ok(vec![
                Source::new("https://excluded.example/search", "tavily"),
                Source::new("https://allowed.example/search", "tavily"),
                Source::new("https://docs.allowed.example/search", "tavily"),
            ])
        }

        async fn fetch(&self, url: &str) -> Result<String> {
            Ok(format!("fetched {url}"))
        }

        async fn map(&self, _url: &str, _max_results: usize) -> Result<Vec<Source>> {
            Ok(Vec::new())
        }
    }

    fn service(ai_sources: Vec<Source>) -> SearchService {
        SearchService::fake_custom(
            Some(Arc::new(MixedDomainAiProvider {
                sources: ai_sources,
            })),
            Arc::new(MixedDomainSourceProvider),
            None,
            [("GROK_SEARCH_EXTRA_SOURCES", "3")],
        )
    }

    #[tokio::test]
    async fn web_search_exclude_domains_filters_grok_enrichment_and_cache() {
        let svc = service(vec![
            Source::new("https://openai.com/news", "grok_responses"),
            Source::new("https://excluded.example/grok", "grok_responses"),
        ]);
        let out = svc
            .web_search(WebSearchInput {
                query: "q".to_string(),
                include_content: Some(false),
                exclude_domains: vec!["https://excluded.example/path".to_string()],
                ..Default::default()
            })
            .await
            .expect("web_search");

        assert!(!out.fallback_used);
        assert!(out
            .sources
            .iter()
            .all(|source| { !source.url.to_ascii_lowercase().contains("excluded.example") }));
        assert!(out
            .sources
            .iter()
            .any(|source| source.url.contains("openai.com")));
        assert!(out
            .sources
            .iter()
            .any(|source| source.url.contains("allowed.example")));

        let cached = svc
            .get_sources(&out.session_id, 0, None)
            .await
            .expect("get_sources");
        assert!(cached
            .sources
            .iter()
            .all(|source| { !source.url.to_ascii_lowercase().contains("excluded.example") }));
    }

    #[tokio::test]
    async fn web_search_include_domains_keeps_domain_and_subdomains_only() {
        let svc = service(vec![
            Source::new("https://allowed.example/grok", "grok_responses"),
            Source::new("https://other.example/grok", "grok_responses"),
        ]);
        let out = svc
            .web_search(WebSearchInput {
                query: "q".to_string(),
                include_content: Some(false),
                include_domains: vec!["allowed.example".to_string()],
                ..Default::default()
            })
            .await
            .expect("web_search");

        assert!(!out.fallback_used);
        assert!(!out.sources.is_empty());
        assert!(out.sources.iter().all(|source| {
            source.url.contains("allowed.example") || source.url.contains("docs.allowed.example")
        }));
        assert!(out
            .sources
            .iter()
            .any(|source| source.url.contains("docs.allowed.example")));
    }

    #[tokio::test]
    async fn web_search_all_grok_sources_filtered_falls_back_with_stable_reason() {
        let svc = service(vec![Source::new(
            "https://blocked.example/grok",
            "grok_responses",
        )]);
        let out = svc
            .web_search(WebSearchInput {
                query: "q".to_string(),
                include_content: Some(false),
                exclude_domains: vec!["blocked.example".to_string()],
                ..Default::default()
            })
            .await
            .expect("web_search");

        assert!(out.fallback_used);
        assert_eq!(
            out.fallback_reason,
            Some("grok_sources_filtered".to_string())
        );
        assert!(out
            .sources
            .iter()
            .all(|source| !source.url.contains("blocked.example")));

        let cached = svc
            .get_sources(&out.session_id, 100, Some(5))
            .await
            .expect("get_sources");
        assert!(cached.sources.is_empty());
        assert_eq!(cached.next_offset, None);
    }

    #[test]
    fn domain_filter_normalizes_urls_case_www_and_subdomains() {
        let filters = SearchFilters {
            recency_days: None,
            include_domains: vec!["https://WWW.OpenAI.com/docs".to_string()],
            exclude_domains: Vec::new(),
        };
        let sources = vec![
            Source::new("https://openai.com/news", "grok_responses"),
            Source::new("https://community.openai.com/t/1", "grok_responses"),
            Source::new("https://evilopenai.com", "grok_responses"),
            Source::new("https://openai.com.evil.test", "grok_responses"),
        ];
        let filtered = filter_sources_by_domains(sources, &filters);
        let urls: Vec<_> = filtered.iter().map(|source| source.url.as_str()).collect();

        assert_eq!(
            urls,
            vec![
                "https://openai.com/news",
                "https://community.openai.com/t/1"
            ]
        );
    }

    #[test]
    fn domain_filter_excludes_subdomains_but_not_suffix_impersonators() {
        let filters = SearchFilters {
            recency_days: None,
            include_domains: Vec::new(),
            exclude_domains: vec!["rust-lang.org".to_string()],
        };
        let sources = vec![
            Source::new("https://docs.rust-lang.org/book", "grok_responses"),
            Source::new("https://rust-lang.org/install", "grok_responses"),
            Source::new("https://rust-lang.org.evil.example", "grok_responses"),
            Source::new("https://notrust-lang.org", "grok_responses"),
        ];
        let filtered = filter_sources_by_domains(sources, &filters);
        let urls: Vec<_> = filtered.iter().map(|source| source.url.as_str()).collect();

        assert_eq!(
            urls,
            vec![
                "https://rust-lang.org.evil.example",
                "https://notrust-lang.org"
            ]
        );
    }

    #[test]
    fn domain_filter_handles_invalid_source_urls_conservatively() {
        let invalid = Source::new("not a url", "grok_responses");
        let exclude_only = SearchFilters {
            recency_days: None,
            include_domains: Vec::new(),
            exclude_domains: vec!["example.com".to_string()],
        };
        assert_eq!(
            filter_sources_by_domains(vec![invalid.clone()], &exclude_only).len(),
            1
        );

        let include_only = SearchFilters {
            recency_days: None,
            include_domains: vec!["example.com".to_string()],
            exclude_domains: Vec::new(),
        };
        assert!(filter_sources_by_domains(vec![invalid], &include_only).is_empty());
    }
}
