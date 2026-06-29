use async_trait::async_trait;
use grok_search_types::model::search::{SearchFilters, SearchRequest, SearchResponse};
use grok_search_types::{
    AcademicCitationSummary, AcademicCitationsOutput, AcademicDownloadPdfOutput, AcademicGetOutput,
    AcademicPaper, AcademicParseOptions, AcademicParsePdfOutput, AcademicReadOutput,
    AcademicSearchInput, AcademicSearchOutput, GrokSearchError, Result, Source, WechatSearchInput,
    WechatSearchOutput, ZhihuSearchInput, ZhihuSearchOutput,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcademicIdentifier {
    Doi(String),
    Arxiv(String),
    Semantic(String),
    OpenAlex(String),
    Dblp(String),
    Url(String),
    Query(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullTextLocation {
    pub url: String,
    pub source: String,
    pub status: String,
}

#[async_trait]
pub trait AiProvider: Send + Sync {
    async fn search(&self, request: &SearchRequest) -> Result<SearchResponse>;
}

#[async_trait]
pub trait SourceProvider: Send + Sync {
    async fn search_sources(
        &self,
        query: &str,
        max_results: usize,
        filters: &SearchFilters,
    ) -> Result<Vec<Source>>;
    async fn fetch(&self, url: &str) -> Result<String>;
    async fn map(&self, url: &str, max_results: usize) -> Result<Vec<Source>>;
}

#[async_trait]
pub trait WechatProvider: Send + Sync {
    async fn search(&self, input: WechatSearchInput) -> Result<WechatSearchOutput>;
}

#[async_trait]
pub trait ZhihuProvider: Send + Sync {
    async fn search(&self, input: ZhihuSearchInput) -> Result<ZhihuSearchOutput>;
}

#[async_trait]
pub trait AcademicProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn search(
        &self,
        _query: &grok_search_types::AcademicSearchInput,
        _limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        Err(GrokSearchError::Provider(format!(
            "{} does not support academic search",
            self.name()
        )))
    }

    async fn get(&self, _identifier: &AcademicIdentifier) -> Result<Option<AcademicPaper>> {
        Ok(None)
    }

    async fn citations(
        &self,
        _identifier: &AcademicIdentifier,
        _limit: usize,
    ) -> Result<Option<AcademicCitationSummary>> {
        Ok(None)
    }

    async fn resolve_fulltext(&self, _paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        Ok(None)
    }
}

#[async_trait]
pub trait AcademicServiceProvider: Send + Sync {
    async fn search(&self, input: AcademicSearchInput) -> Result<AcademicSearchOutput>;

    async fn get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
        extract_material_links: bool,
    ) -> Result<AcademicGetOutput>;

    async fn citations(&self, identifier: &str, limit: usize) -> Result<AcademicCitationsOutput>;

    async fn read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicReadOutput>;

    async fn parse_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicParsePdfOutput>;

    async fn download_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        output_path: String,
        overwrite: bool,
    ) -> Result<AcademicDownloadPdfOutput>;

    fn diagnostics(&self) -> serde_json::Value;

    async fn diagnostics_live(&self) -> serde_json::Value {
        self.diagnostics()
    }

    fn warm_institutional_access(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named;

    #[async_trait]
    impl AcademicProvider for Named {
        fn name(&self) -> &'static str {
            "named"
        }
    }

    #[tokio::test]
    async fn default_academic_capability_error_names_provider() {
        let err = Named
            .search(&grok_search_types::AcademicSearchInput::default(), 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("named"));
        assert!(err.contains("does not support academic search"));
    }
}
