use crate::adapters::grok_responses_request::to_grok_responses_payload;
use crate::adapters::grok_responses_response::parse_grok_responses;
use grok_search_auth::{CredentialProvider, StaticApiKeyCredential};
use grok_search_net::http::{build_client, post_json_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_provider_core::AiProvider;
use grok_search_types::model::search::{SearchRequest, SearchResponse};
use grok_search_types::Result;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct GrokResponsesProvider {
    client: Client,
    api_url: String,
    credential: Arc<dyn CredentialProvider>,
    require_web_search: bool,
    include_x_search: bool,
    max_response_bytes: usize,
}

impl GrokResponsesProvider {
    pub fn new(
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        require_web_search: bool,
        include_x_search: bool,
        timeout: Duration,
    ) -> Self {
        Self::with_client(
            build_client(timeout),
            api_url,
            api_key,
            require_web_search,
            include_x_search,
        )
    }

    /// Construct with an externally provided `reqwest::Client`. Used by
    /// `SearchService::new` to share one tuned client across providers; the
    /// `new(.., timeout)` form remains for callers that prefer per-provider
    /// timeouts (tests, integration users).
    pub fn with_client(
        client: Client,
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        require_web_search: bool,
        include_x_search: bool,
    ) -> Self {
        Self::with_client_and_limit(
            client,
            api_url,
            api_key,
            require_web_search,
            include_x_search,
            DEFAULT_MAX_RESPONSE_BYTES,
        )
    }

    pub fn with_client_and_limit(
        client: Client,
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        require_web_search: bool,
        include_x_search: bool,
        max_response_bytes: usize,
    ) -> Self {
        Self::with_credential_client_and_limit(
            client,
            api_url,
            Arc::new(StaticApiKeyCredential::new(api_key.into())),
            require_web_search,
            include_x_search,
            max_response_bytes,
        )
    }

    pub fn with_credential_client(
        client: Client,
        api_url: impl Into<String>,
        credential: Arc<dyn CredentialProvider>,
        require_web_search: bool,
        include_x_search: bool,
    ) -> Self {
        Self::with_credential_client_and_limit(
            client,
            api_url,
            credential,
            require_web_search,
            include_x_search,
            DEFAULT_MAX_RESPONSE_BYTES,
        )
    }

    pub fn with_credential_client_and_limit(
        client: Client,
        api_url: impl Into<String>,
        credential: Arc<dyn CredentialProvider>,
        require_web_search: bool,
        include_x_search: bool,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            client,
            api_url: api_url.into().trim_end_matches('/').to_string(),
            credential,
            require_web_search,
            include_x_search,
            max_response_bytes,
        }
    }

    pub fn endpoint(&self) -> String {
        format!("{}/responses", self.api_url)
    }

    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        let payload =
            to_grok_responses_payload(request, self.require_web_search, self.include_x_search)?;
        let token = self.credential.bearer_token().await?;
        let raw = post_json_limited(
            &self.client,
            &self.endpoint(),
            &token,
            &payload,
            "Grok Responses",
            self.max_response_bytes,
        )
        .await?;
        parse_grok_responses(&raw)
    }
}

#[async_trait::async_trait]
impl AiProvider for GrokResponsesProvider {
    async fn search(&self, request: &SearchRequest) -> Result<SearchResponse> {
        GrokResponsesProvider::search(self, request).await
    }
}
