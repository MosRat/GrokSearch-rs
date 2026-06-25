use async_trait::async_trait;
use reqwest::Client;
use url::Url;

use grok_search_types::Result;
pub use grok_search_types::SourceType;

pub const NO_SPECIALIST_MATCH: &str = "no_specialist_match";

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

#[async_trait]
pub trait SourceExtractor: Send + Sync {
    fn matches(&self, url: &Url) -> bool;
    fn kind(&self) -> SourceType;
    async fn fetch_render(&self, client: &Client, url: &Url, caps: &SourceCaps) -> Result<String>;
}

#[derive(Default)]
pub struct SourceRouter {
    extractors: Vec<Box<dyn SourceExtractor>>,
}

impl SourceRouter {
    pub fn with_extractors(extractors: Vec<Box<dyn SourceExtractor>>) -> Self {
        Self { extractors }
    }

    pub fn find<'a>(&'a self, url: &Url) -> Option<&'a dyn SourceExtractor> {
        self.extractors
            .iter()
            .find(|e| e.matches(url))
            .map(|e| e.as_ref())
    }
}

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
    use grok_search_types::GrokSearchError;

    struct Always(SourceType);

    #[async_trait]
    impl SourceExtractor for Always {
        fn matches(&self, _url: &Url) -> bool {
            true
        }

        fn kind(&self) -> SourceType {
            self.0.clone()
        }

        async fn fetch_render(
            &self,
            _client: &Client,
            _url: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            Ok("ok".to_string())
        }
    }

    struct Fails;

    #[async_trait]
    impl SourceExtractor for Fails {
        fn matches(&self, _url: &Url) -> bool {
            true
        }

        fn kind(&self) -> SourceType {
            SourceType::GithubIssue
        }

        async fn fetch_render(
            &self,
            _client: &Client,
            _url: &Url,
            _caps: &SourceCaps,
        ) -> Result<String> {
            Err(GrokSearchError::Provider("boom".into()))
        }
    }

    #[test]
    fn router_returns_first_match() {
        let router = SourceRouter::with_extractors(vec![Box::new(Always(SourceType::Arxiv))]);
        let url = Url::parse("https://example.com").unwrap();
        assert_eq!(router.find(&url).unwrap().kind(), SourceType::Arxiv);
    }

    #[tokio::test]
    async fn no_match_returns_sentinel() {
        let router = SourceRouter::default();
        let url = Url::parse("https://example.com").unwrap();
        let err = resolve_content(&Client::new(), &url, &router, &SourceCaps::default())
            .await
            .unwrap_err();
        assert_eq!(err, NO_SPECIALIST_MATCH);
    }

    #[tokio::test]
    async fn failed_extractor_is_labeled() {
        let router = SourceRouter::with_extractors(vec![Box::new(Fails)]);
        let url = Url::parse("https://example.com").unwrap();
        let err = resolve_content(&Client::new(), &url, &router, &SourceCaps::default())
            .await
            .unwrap_err();
        assert!(err.starts_with("github_issue"));
    }
}
