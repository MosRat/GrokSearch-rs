use async_trait::async_trait;
use grok_search_provider_core::{AcademicProvider, FullTextLocation};
use grok_search_types::{AcademicPaper, Result};

#[derive(Clone)]
pub(crate) struct SciHubProvider {
    enabled: bool,
    base_url: Option<String>,
}

impl SciHubProvider {
    pub(crate) fn new(_client: reqwest::Client, enabled: bool, base_url: Option<String>) -> Self {
        Self { enabled, base_url }
    }
}

#[async_trait]
impl AcademicProvider for SciHubProvider {
    fn name(&self) -> &'static str {
        "scihub"
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        if !self.enabled {
            return Ok(None);
        }
        let Some(base) = &self.base_url else {
            return Ok(None);
        };
        let Some(identifier) = paper.doi.as_ref().or(paper.arxiv_id.as_ref()) else {
            return Ok(None);
        };
        let base = base.trim_end_matches('/');
        Ok(Some(FullTextLocation {
            url: format!("{base}/{identifier}"),
            source: "scihub".to_string(),
            status: "scihub_fallback_enabled_user_responsibility".to_string(),
        }))
    }
}
