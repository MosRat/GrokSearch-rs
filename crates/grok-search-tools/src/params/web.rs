use grok_search_types::model::tool::WebSearchInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    pub query: String,
    pub platform: Option<String>,
    pub model: Option<String>,
    pub extra_sources: Option<usize>,
    pub recency_days: Option<u32>,
    #[serde(default)]
    pub include_domains: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
    pub include_content: Option<bool>,
    pub response_format: Option<String>,
}

impl From<WebSearchParams> for WebSearchInput {
    fn from(params: WebSearchParams) -> Self {
        Self {
            query: params.query,
            platform: params.platform,
            model: params.model,
            extra_sources: params.extra_sources,
            recency_days: params.recency_days.filter(|value| *value > 0),
            include_domains: params.include_domains,
            exclude_domains: params.exclude_domains,
            include_content: params.include_content,
            response_format: params.response_format,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSourcesParams {
    pub session_id: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebFetchParams {
    pub url: String,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebMapParams {
    pub url: String,
    pub max_results: Option<usize>,
}
