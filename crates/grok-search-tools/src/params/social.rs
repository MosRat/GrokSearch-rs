use grok_search_types::model::tool::{WechatSearchInput, ZhihuSearchInput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WechatSearchParams {
    pub query: String,
    pub account: Option<String>,
    pub max_results: Option<usize>,
    pub pages: Option<usize>,
    pub include_content: Option<bool>,
    pub max_content_chars: Option<usize>,
}

impl From<WechatSearchParams> for WechatSearchInput {
    fn from(params: WechatSearchParams) -> Self {
        Self {
            query: params.query,
            account: params.account,
            max_results: params.max_results,
            pages: params.pages,
            include_content: params.include_content,
            max_content_chars: params.max_content_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZhihuSearchParams {
    pub query: String,
    pub count: Option<usize>,
}

impl From<ZhihuSearchParams> for ZhihuSearchInput {
    fn from(params: ZhihuSearchParams) -> Self {
        Self {
            query: params.query,
            count: params.count,
        }
    }
}
