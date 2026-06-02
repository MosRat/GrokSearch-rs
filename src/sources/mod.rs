use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Result;

/// Sentinel `Err` value returned by [`resolve_content`] when no specialist
/// extractor matched the URL. The service layer treats this as "go generic
/// silently" — no `fallback_reason` is surfaced (per decision D-01), because no
/// specialist was ever attempted.
pub const NO_SPECIALIST_MATCH: &str = "no_specialist_match";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    GithubIssue,
    GithubPull,
    Stackexchange,
    Arxiv,
    Wikipedia,
    Generic,
}

impl SourceType {
    /// Static label, identical to the serde representation. Used to build
    /// concise machine-readable `fallback_reason` strings without allocating.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::GithubIssue => "github_issue",
            SourceType::GithubPull => "github_pull",
            SourceType::Stackexchange => "stackexchange",
            SourceType::Arxiv => "arxiv",
            SourceType::Wikipedia => "wikipedia",
            SourceType::Generic => "generic",
        }
    }
}

/// Per-request rendering caps passed to extractors. Phase 1 only carries the
/// defaults; Phase 2 extractors honor them when folding long comment/answer
/// lists.
#[derive(Debug, Clone)]
pub struct SourceCaps {
    pub max_answers: usize,
    pub max_comments: usize,
}

impl Default for SourceCaps {
    fn default() -> Self {
        Self {
            max_answers: 5,
            max_comments: 30,
        }
    }
}

/// A specialist content extractor for one family of URLs (GitHub, arXiv, ...).
/// Object-safe: every method takes `&self` so the router can hold
/// `Box<dyn SourceExtractor>` and dispatch dynamically.
#[async_trait]
pub trait SourceExtractor: Send + Sync {
    /// Cheap, synchronous URL test — must take `&self` for object safety.
    fn matches(&self, url: &Url) -> bool;
    /// The `source_type` this extractor produces on success.
    fn kind(&self) -> SourceType;
    /// Fetch and render structured Markdown for `url`. Returning `Ok` with
    /// empty/whitespace content is treated as a failure by `resolve_content`.
    async fn fetch_render(&self, client: &Client, url: &Url, caps: &SourceCaps) -> Result<String>;
}

/// Static, ordered list of extractors (decision D-05). First `matches` hit wins.
/// Read-only after construction.
pub struct SourceRouter {
    extractors: Vec<Box<dyn SourceExtractor>>,
}

impl Default for SourceRouter {
    /// Production constructor. Phase 1 has no concrete extractors, so this is
    /// empty — every `find` returns `None` and `web_fetch` always falls back to
    /// the generic chain. Phase 2 populates it.
    fn default() -> Self {
        Self {
            extractors: Vec::new(),
        }
    }
}

impl SourceRouter {
    /// Injection constructor for tests and future wiring.
    pub fn with_extractors(extractors: Vec<Box<dyn SourceExtractor>>) -> Self {
        Self { extractors }
    }

    /// First extractor whose `matches(url)` is true, or `None`.
    pub fn find<'a>(&'a self, url: &Url) -> Option<&'a dyn SourceExtractor> {
        self.extractors
            .iter()
            .find(|e| e.matches(url))
            .map(|e| e.as_ref())
    }
}

/// Unified content resolution (decision D-06). Tries the first matching
/// specialist extractor; on success returns `(markdown, source_type)`.
///
/// The `Err` variant is a human-readable string, **not** a `GrokSearchError` —
/// a fallback is not a service error, it is a routing outcome. The service layer
/// inspects this:
/// - `Err(NO_SPECIALIST_MATCH)` → go generic silently (no `fallback_reason`).
/// - any other `Err(reason)` → go generic and surface `reason` as
///   `fallback_reason` (a specialist matched but failed or rendered empty).
pub async fn resolve_content(
    client: &Client,
    url: &Url,
    router: &SourceRouter,
    caps: &SourceCaps,
) -> std::result::Result<(String, SourceType), String> {
    let Some(extractor) = router.find(url) else {
        return Err(NO_SPECIALIST_MATCH.to_string());
    };
    match extractor.fetch_render(client, url, caps).await {
        Ok(content) if !content.trim().is_empty() => Ok((content, extractor.kind())),
        Ok(_) => Err(format!("{} empty render", extractor.kind().as_str())),
        Err(e) => Err(format!("{} {}", extractor.kind().as_str(), e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{GrokSearchError, Result};
    use reqwest::Client;
    use url::Url;

    struct AlwaysMatch;
    #[async_trait::async_trait]
    impl SourceExtractor for AlwaysMatch {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::GithubIssue
        }
        async fn fetch_render(&self, _c: &Client, _u: &Url, _caps: &SourceCaps) -> Result<String> {
            Ok("ok".to_string())
        }
    }

    struct NeverMatch;
    #[async_trait::async_trait]
    impl SourceExtractor for NeverMatch {
        fn matches(&self, _url: &Url) -> bool {
            false
        }
        fn kind(&self) -> SourceType {
            SourceType::Wikipedia
        }
        async fn fetch_render(&self, _c: &Client, _u: &Url, _caps: &SourceCaps) -> Result<String> {
            Ok("never".to_string())
        }
    }

    struct AlwaysErr;
    #[async_trait::async_trait]
    impl SourceExtractor for AlwaysErr {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::GithubIssue
        }
        async fn fetch_render(&self, _c: &Client, _u: &Url, _caps: &SourceCaps) -> Result<String> {
            Err(GrokSearchError::Provider("boom".to_string()))
        }
    }

    struct AlwaysEmpty;
    #[async_trait::async_trait]
    impl SourceExtractor for AlwaysEmpty {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::GithubIssue
        }
        async fn fetch_render(&self, _c: &Client, _u: &Url, _caps: &SourceCaps) -> Result<String> {
            Ok("   \n  ".to_string()) // whitespace only → empty per D-04
        }
    }

    struct AlwaysOk;
    #[async_trait::async_trait]
    impl SourceExtractor for AlwaysOk {
        fn matches(&self, _url: &Url) -> bool {
            true
        }
        fn kind(&self) -> SourceType {
            SourceType::Wikipedia
        }
        async fn fetch_render(&self, _c: &Client, _u: &Url, _caps: &SourceCaps) -> Result<String> {
            Ok("hello world".to_string())
        }
    }

    fn test_url() -> Url {
        Url::parse("https://example.com/x").unwrap()
    }

    #[test]
    fn source_type_as_str_matches_serialization() {
        assert_eq!(SourceType::GithubPull.as_str(), "github_pull");
        assert_eq!(SourceType::Generic.as_str(), "generic");
    }

    #[test]
    fn source_caps_default_is_5_answers_30_comments() {
        let caps = SourceCaps::default();
        assert_eq!(caps.max_answers, 5);
        assert_eq!(caps.max_comments, 30);
    }

    #[test]
    fn empty_router_finds_nothing() {
        let router = SourceRouter::default();
        let url = Url::parse("https://github.com/o/r/issues/1").unwrap();
        assert!(router.find(&url).is_none());
    }

    #[test]
    fn router_with_extractor_finds_first_match() {
        let router = SourceRouter::with_extractors(vec![Box::new(AlwaysMatch)]);
        let url = Url::parse("https://example.com/").unwrap();
        let found = router.find(&url).expect("extractor should match");
        assert_eq!(found.kind(), SourceType::GithubIssue);
    }

    #[test]
    fn router_returns_first_matching_extractor_in_order() {
        // NeverMatch is skipped; the first AlwaysMatch (GithubIssue) wins over a
        // later extractor, proving sequential first-hit semantics (D-05).
        let router = SourceRouter::with_extractors(vec![
            Box::new(NeverMatch),
            Box::new(AlwaysMatch),
        ]);
        let url = Url::parse("https://example.com/").unwrap();
        let found = router.find(&url).expect("second extractor should match");
        assert_eq!(found.kind(), SourceType::GithubIssue);
    }

    #[test]
    fn source_type_serializes_to_required_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&SourceType::GithubIssue).unwrap(),
            "\"github_issue\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::GithubPull).unwrap(),
            "\"github_pull\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Stackexchange).unwrap(),
            "\"stackexchange\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Arxiv).unwrap(),
            "\"arxiv\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Wikipedia).unwrap(),
            "\"wikipedia\""
        );
        assert_eq!(
            serde_json::to_string(&SourceType::Generic).unwrap(),
            "\"generic\""
        );
    }

    #[tokio::test]
    async fn resolve_content_no_match_returns_sentinel() {
        let client = Client::new();
        let router = SourceRouter::default();
        let err = resolve_content(&client, &test_url(), &router, &SourceCaps::default())
            .await
            .unwrap_err();
        assert_eq!(err, NO_SPECIALIST_MATCH);
    }

    #[tokio::test]
    async fn resolve_content_err_returns_labeled_reason() {
        let client = Client::new();
        let router = SourceRouter::with_extractors(vec![Box::new(AlwaysErr)]);
        let err = resolve_content(&client, &test_url(), &router, &SourceCaps::default())
            .await
            .unwrap_err();
        assert!(err.starts_with("github_issue"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_content_empty_render_falls_back() {
        let client = Client::new();
        let router = SourceRouter::with_extractors(vec![Box::new(AlwaysEmpty)]);
        let err = resolve_content(&client, &test_url(), &router, &SourceCaps::default())
            .await
            .unwrap_err();
        assert!(err.contains("empty render"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_content_ok_returns_content_and_kind() {
        let client = Client::new();
        let router = SourceRouter::with_extractors(vec![Box::new(AlwaysOk)]);
        let (content, kind) =
            resolve_content(&client, &test_url(), &router, &SourceCaps::default())
                .await
                .unwrap();
        assert_eq!(content, "hello world");
        assert_eq!(kind, SourceType::Wikipedia);
    }
}
