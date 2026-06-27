use std::time::Instant;

use serde_json::json;

use crate::service::{Probe, SearchService};
use grok_search_config::AuthMode;
use grok_search_types::model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchTool,
};

impl SearchService {
    pub async fn doctor(&self) -> serde_json::Value {
        self.doctor_with_options(false).await
    }

    pub async fn doctor_with_options(&self, verbose: bool) -> serde_json::Value {
        use grok_search_config::Transport;

        let request_id = self.logger.request_id();
        let start = Instant::now();
        self.logger.event(
            &request_id,
            "debug",
            "doctor.start",
            Some("doctor"),
            None,
            json!({ "verbose": verbose }),
        );
        let grok_probe = self.probe_grok().await;
        let tavily_probe = match &self.sources {
            Some(provider) => probe_source(provider.as_ref(), "https://example.com").await,
            None => Probe::skipped("TAVILY_API_KEY not configured"),
        };
        let firecrawl_probe = match &self.fallback_sources {
            Some(provider) => probe_source(provider.as_ref(), "https://example.com").await,
            None => Probe::skipped("FIRECRAWL_API_KEY not configured"),
        };

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
                false,
            ),
        };

        let mut report = serde_json::json!({
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
            "academic": match &self.academic {
                Some(academic) => academic.diagnostics_live().await,
                None => serde_json::json!({ "enabled": false }),
            },
            "redacted": self.config.redacted_diagnostics()
        });

        if verbose {
            report["diagnostics"] = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "debug_log": {
                    "enabled": self.logger.enabled(),
                    "path": self.logger.path().map(|path| path.display().to_string()),
                    "session_id": self.logger.session_id(),
                },
                "limits": {
                    "timeout_seconds": self.config.timeout.as_secs(),
                    "max_response_bytes": self.config.max_response_bytes,
                    "response_max_chars": self.config.response_max_chars,
                    "fetch_max_chars": self.config.fetch_max_chars,
                    "academic_max_pdf_bytes": self.config.academic_max_pdf_bytes,
                    "academic_pdf_max_chars": self.config.academic_pdf_max_chars,
                },
                "url_policy": {
                    "web_tools": "public http/https only; localhost, private, link-local, multicast, and unspecified addresses are rejected",
                    "academic_institutional": "public targets require HTTPS and valid TLS; private/local IEEE/ACM targets may use HTTP or invalid TLS",
                },
                "providers": {
                    "ai": {
                        "configured": true,
                        "transport": provider_label,
                        "api_url": ai_api_url,
                    },
                    "tavily": {
                        "configured": self.sources.is_some(),
                        "enabled": self.config.tavily_enabled,
                    },
                    "firecrawl": {
                        "configured": self.fallback_sources.is_some(),
                        "enabled": self.config.firecrawl_enabled,
                    },
                },
            });
        }
        self.logger.event(
            &request_id,
            "debug",
            "doctor.success",
            Some("doctor"),
            Some(start.elapsed()),
            json!({ "verbose": verbose }),
        );
        report
    }

    async fn probe_grok(&self) -> Probe {
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
}

async fn probe_source(provider: &dyn crate::service::SourceProvider, sample_url: &str) -> Probe {
    let filters = SearchFilters::default();
    match provider.search_sources("ping", 1, &filters).await {
        Ok(_) => Probe::ok(format!("reachable (sample probe via {sample_url} ok)")),
        Err(err) => Probe::failed(err.to_string()),
    }
}
