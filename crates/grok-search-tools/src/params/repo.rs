use grok_search_types::{RepoMetadataInput, RepoProvider};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RepoMetadataParams {
    pub url: Option<String>,
    pub provider: Option<RepoProviderParam>,
    pub repo_id: Option<String>,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_type: Option<String>,
    pub include_readme: Option<bool>,
    pub include_card: Option<bool>,
    pub max_text_chars: Option<usize>,
}

impl From<RepoMetadataParams> for RepoMetadataInput {
    fn from(params: RepoMetadataParams) -> Self {
        Self {
            url: params.url,
            provider: params.provider.map(Into::into),
            repo_id: params.repo_id,
            owner: params.owner,
            name: params.name,
            repo_type: params.repo_type,
            include_readme: params.include_readme,
            include_card: params.include_card,
            max_text_chars: params.max_text_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepoProviderParam {
    Github,
    Huggingface,
}

impl From<RepoProviderParam> for RepoProvider {
    fn from(value: RepoProviderParam) -> Self {
        match value {
            RepoProviderParam::Github => RepoProvider::Github,
            RepoProviderParam::Huggingface => RepoProvider::Huggingface,
        }
    }
}
