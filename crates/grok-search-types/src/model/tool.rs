use serde::{Deserialize, Serialize};

use crate::model::source::Source;
use crate::SourceType;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebSearchInput {
    pub query: String,
    pub platform: Option<String>,
    pub model: Option<String>,
    pub extra_sources: Option<usize>,
    pub recency_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_domains: Vec<String>,
    pub include_content: Option<bool>,
    /// `"concise"` (answer + source metadata only) or `"detailed"` (inline
    /// content, subject to the response budget). When set, takes precedence
    /// over the legacy `include_content` flag.
    pub response_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchOutput {
    pub session_id: String,
    pub content: String,
    pub sources_count: usize,
    pub sources: Vec<Source>,
    pub search_provider: String,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
    /// True when the response budget trimmed inline source content. The cache
    /// keeps the full text — recover via `get_sources` or `web_fetch`.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSourcesOutput {
    pub session_id: String,
    pub sources: Vec<Source>,
    /// Number of sources in THIS page (`sources.len()`), not the cache total.
    pub sources_count: usize,
    /// Total sources cached for the session, across all pages.
    pub total_sources: usize,
    pub offset: usize,
    /// Offset of the next page; absent when this page reaches the end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    /// True when the response budget trimmed inline content within this page.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebFetchOutput {
    pub url: String,
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    /// Always present. `generic` on the fallback path (decision D-02);
    /// specialist values arrive in Phase 2.
    pub source_type: SourceType,
    /// Present only when a specialist was matched and then failed
    /// (decision D-01); omitted from JSON otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WechatSearchInput {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub max_results: Option<usize>,
    pub pages: Option<usize>,
    pub include_content: Option<bool>,
    pub max_content_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WechatSearchOutput {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub articles_count: usize,
    pub articles: Vec<WechatArticle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WechatArticle {
    pub title: String,
    pub snippet: String,
    pub source: String,
    pub published_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub sogou_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_original_length: Option<usize>,
    pub content_truncated: bool,
    pub quality: WechatArticleQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WechatArticleQuality {
    pub source_match: bool,
    pub url_resolved: bool,
    pub content_fetched: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ZhihuSearchInput {
    pub query: String,
    pub count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZhihuSearchOutput {
    pub query: String,
    pub code: i64,
    pub message: String,
    pub item_count: usize,
    pub items: Vec<ZhihuSearchItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZhihuSearchItem {
    pub title: String,
    pub summary: String,
    pub url: String,
    pub author_name: String,
    pub vote_up_count: u64,
    pub comment_count: u64,
    pub edit_time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoProvider {
    Github,
    Huggingface,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoKind {
    GithubRepository,
    HuggingfaceModel,
    HuggingfaceDataset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepoMetadataInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<RepoProvider>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_readme: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_card: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_text_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepoLinks {
    pub html: String,
    pub api: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoText {
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoMetadataOutput {
    pub provider: RepoProvider,
    pub kind: RepoKind,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub likes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    pub links: RepoLinks,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme: Option<RepoText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<RepoText>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceType;

    #[test]
    fn web_fetch_output_includes_source_type_and_omits_none_fallback_reason() {
        let output = WebFetchOutput {
            url: "https://example.com".to_string(),
            content: "hi".to_string(),
            original_length: 2,
            truncated: false,
            source_type: SourceType::Generic,
            fallback_reason: None,
        };
        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["source_type"], "generic");
        assert!(value.get("fallback_reason").is_none());
    }

    #[test]
    fn web_fetch_output_serializes_fallback_reason_when_present() {
        let output = WebFetchOutput {
            url: "https://example.com".to_string(),
            content: "hi".to_string(),
            original_length: 2,
            truncated: false,
            source_type: SourceType::Generic,
            fallback_reason: Some("github_issue empty render".to_string()),
        };
        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["fallback_reason"], "github_issue empty render");
    }

    #[test]
    fn web_fetch_output_serializes_github_repo_source_type() {
        let output = WebFetchOutput {
            url: "https://github.com/owner/repo".to_string(),
            content: "# owner/repo".to_string(),
            original_length: 12,
            truncated: false,
            source_type: SourceType::GithubRepo,
            fallback_reason: None,
        };
        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["source_type"], "github_repo");
        assert_eq!(SourceType::GithubRepo.as_str(), "github_repo");
    }

    #[test]
    fn repo_metadata_output_omits_absent_optional_text() {
        let output = RepoMetadataOutput {
            provider: RepoProvider::Github,
            kind: RepoKind::GithubRepository,
            id: "owner/repo".to_string(),
            name: "repo".to_string(),
            owner: Some("owner".to_string()),
            description: None,
            license: None,
            tags: Vec::new(),
            stars: Some(1),
            forks: Some(2),
            downloads: None,
            likes: None,
            created_at: None,
            updated_at: None,
            default_branch: Some("main".to_string()),
            links: RepoLinks {
                html: "https://github.com/owner/repo".to_string(),
                api: "https://api.github.com/repos/owner/repo".to_string(),
                readme: None,
                card: None,
            },
            readme: None,
            card: None,
            warnings: Vec::new(),
        };
        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["provider"], "github");
        assert_eq!(value["kind"], "github_repository");
        assert!(value.get("readme").is_none());
        assert!(value.get("warnings").is_none());
    }
}
