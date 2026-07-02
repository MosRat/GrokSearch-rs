use std::sync::Arc;

use grok_search_academic::AcademicService;
use grok_search_auth::{OAuthCredential, StaticApiKeyCredential};
use grok_search_config::{AuthMode, Config, Transport};
use grok_search_net::proxy::ProxyDiagnostics;
use grok_search_provider_core::{AiProvider, SourceProvider, WechatProvider, ZhihuProvider};
use grok_search_providers::providers::firecrawl::FirecrawlProvider;
use grok_search_providers::providers::grok::GrokResponsesProvider;
use grok_search_providers::providers::openai_compatible::OpenAICompatProvider;
use grok_search_providers::providers::tavily::TavilyProvider;
use grok_search_providers::providers::wechat::WechatSearchProvider;
use grok_search_providers::providers::zhihu::ZhihuSearchProvider;
use grok_search_service::{SearchService, SearchServiceParts};
use grok_search_types::{GrokSearchError, Result};

pub fn new(config: Config) -> Result<SearchService> {
    let http = grok_search_net::http::build_client(config.timeout)?;
    new_with_http(config, http, ProxyDiagnostics::default())
}

pub fn new_with_http(
    config: Config,
    http: reqwest::Client,
    proxy_diagnostics: ProxyDiagnostics,
) -> Result<SearchService> {
    let ai = ai_provider(&config, &http)?;
    let sources = source_provider(&config, &http);
    let fallback_sources = fallback_source_provider(&config, &http);
    let source_router = grok_search_sources::sources::router_from_config(&config);
    let academic = config.academic_enabled.then(|| {
        Arc::new(AcademicService::new(http.clone(), config.clone()))
            as Arc<dyn grok_search_provider_core::AcademicServiceProvider>
    });
    let wechat = wechat_provider(&config, &http);
    let zhihu = zhihu_provider(&config, &http);

    Ok(SearchService::from_parts(SearchServiceParts {
        config,
        ai,
        sources,
        fallback_sources,
        http_client: http,
        source_router,
        proxy_diagnostics,
        academic,
        wechat,
        zhihu,
    }))
}

fn ai_provider(config: &Config, http: &reqwest::Client) -> Result<Arc<dyn AiProvider>> {
    match config.transport {
        Transport::Responses => {
            let credential: Arc<dyn grok_search_auth::CredentialProvider> =
                match config.grok_auth_mode {
                    AuthMode::ApiKey => Arc::new(StaticApiKeyCredential::new(
                        config
                            .grok_api_key
                            .clone()
                            .ok_or(GrokSearchError::MissingConfig("GROK_SEARCH_API_KEY"))?,
                    )),
                    AuthMode::OAuth => {
                        let auth_path = config
                            .grok_auth_file
                            .clone()
                            .or_else(grok_search_config::auth_path)
                            .ok_or_else(|| {
                                GrokSearchError::OAuth(
                                    "oauth_auth_path_unavailable: set GROK_SEARCH_AUTH_FILE"
                                        .to_string(),
                                )
                            })?;
                        Arc::new(OAuthCredential::new(http.clone(), auth_path))
                    }
                };
            Ok(Arc::new(
                GrokResponsesProvider::with_credential_client_and_limit(
                    http.clone(),
                    config.grok_api_url.clone(),
                    credential,
                    config.web_search_enabled,
                    config.x_search_enabled,
                    config.max_response_bytes,
                ),
            ))
        }
        Transport::ChatCompletions => {
            let url = config
                .openai_compatible_api_url
                .clone()
                .ok_or(GrokSearchError::MissingConfig("OPENAI_COMPATIBLE_API_URL"))?;
            let key = config
                .openai_compatible_api_key
                .clone()
                .ok_or(GrokSearchError::MissingConfig("OPENAI_COMPATIBLE_API_KEY"))?;
            let model = config
                .openai_compatible_model
                .clone()
                .unwrap_or_else(|| config.grok_model.clone());
            if config.x_search_enabled {
                tracing::warn!(
                    target: "grok_search",
                    "x_search_enabled is ignored when using OPENAI_COMPATIBLE_* transport"
                );
            }
            Ok(Arc::new(OpenAICompatProvider::with_client_and_limit(
                http.clone(),
                url,
                key,
                model,
                config.web_search_enabled,
                config.max_response_bytes,
            )))
        }
    }
}

fn source_provider(config: &Config, http: &reqwest::Client) -> Option<Arc<dyn SourceProvider>> {
    if !config.tavily_enabled {
        return None;
    }
    config.tavily_api_key.clone().map(|key| {
        Arc::new(TavilyProvider::with_client_and_limit(
            http.clone(),
            config.tavily_api_url.clone(),
            key,
            config.max_response_bytes,
        )) as Arc<dyn SourceProvider>
    })
}

fn fallback_source_provider(
    config: &Config,
    http: &reqwest::Client,
) -> Option<Arc<dyn SourceProvider>> {
    if !config.firecrawl_enabled {
        return None;
    }
    config.firecrawl_api_key.clone().map(|key| {
        Arc::new(FirecrawlProvider::with_client_and_limit(
            http.clone(),
            config.firecrawl_api_url.clone(),
            key,
            config.max_response_bytes,
        )) as Arc<dyn SourceProvider>
    })
}

fn wechat_provider(config: &Config, http: &reqwest::Client) -> Option<Arc<dyn WechatProvider>> {
    Some(Arc::new(WechatSearchProvider::with_client_and_limit(
        http.clone(),
        config.max_response_bytes,
    )) as Arc<dyn WechatProvider>)
}

fn zhihu_provider(config: &Config, http: &reqwest::Client) -> Option<Arc<dyn ZhihuProvider>> {
    config.zhihu_api_key.clone().map(|key| {
        Arc::new(ZhihuSearchProvider::with_client_base_url_and_limit(
            http.clone(),
            key,
            config.zhihu_openapi_base_url.clone(),
            config.zhihu_search_url.clone(),
            config.max_response_bytes,
        )) as Arc<dyn ZhihuProvider>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_requires_grok_key_for_responses_transport() {
        let config = Config::from_env_map([] as [(&str, &str); 0]);
        let err = match new(config) {
            Ok(_) => panic!("runtime should require GROK_SEARCH_API_KEY"),
            Err(err) => err.to_string(),
        };
        assert!(err.contains("GROK_SEARCH_API_KEY"), "got: {err}");
    }

    #[test]
    fn runtime_wires_responses_transport_with_optional_sources() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("TAVILY_API_KEY", "tvly-fake"),
            ("FIRECRAWL_API_KEY", "fc-fake"),
            ("GROK_SEARCH_AUDIT_ENABLED", "false"),
        ]);
        let _service = new(config).expect("runtime should assemble service");
    }

    #[test]
    fn runtime_wires_chat_completions_transport() {
        let config = Config::from_env_map([
            ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
            ("OPENAI_COMPATIBLE_MODEL", "gpt-4o-mini"),
            ("TAVILY_API_KEY", "fake-tavily"),
            ("GROK_SEARCH_AUDIT_ENABLED", "false"),
        ]);
        let _service = new(config).expect("runtime should assemble chat transport");
    }
}
