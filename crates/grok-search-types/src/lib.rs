pub mod error;
pub mod model;

pub use error::{GrokSearchError, Result};
pub use model::academic::{
    AcademicCitationSummary, AcademicCitationsInput, AcademicCitationsOutput,
    AcademicDownloadPdfInput, AcademicDownloadPdfOutput, AcademicGetInput, AcademicGetOutput,
    AcademicLlmProgressiveOptions, AcademicMaterialLink, AcademicPaper, AcademicParseArtifact,
    AcademicParseCapabilities, AcademicParseOptions, AcademicParsePdfInput, AcademicParsePdfOutput,
    AcademicPdfArtifactsInput, AcademicPdfArtifactsOutput, AcademicPdfCacheInfo,
    AcademicPdfCachePolicy, AcademicPdfDownloadInput, AcademicPdfDownloadOutput,
    AcademicPdfLocator, AcademicPdfPassReport, AcademicPdfProcessingReport, AcademicPdfReadInput,
    AcademicPdfReadOutput, AcademicPdfStructureInput, AcademicPdfStructureOutput,
    AcademicPdfStructureProfile, AcademicProgressiveBudget, AcademicProgressiveCacheInfo,
    AcademicProgressiveChunkReport, AcademicProgressiveEvidenceSpan, AcademicProgressiveFigure,
    AcademicProgressiveGetInput, AcademicProgressiveGetOutput, AcademicProgressiveLlmReport,
    AcademicProgressiveMetadata, AcademicProgressiveOutlineNode, AcademicProgressivePaper,
    AcademicProgressiveReference, AcademicProgressiveSection, AcademicProgressiveTable,
    AcademicReadInput, AcademicReadOutput, AcademicSearchInput, AcademicSearchOutput,
};
pub use model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchResponse, SearchTool,
};
pub use model::source::{merge_sources, Source};
pub use model::tool::{
    GetSourcesOutput, RepoKind, RepoLinks, RepoMetadataInput, RepoMetadataOutput, RepoProvider,
    RepoText, WebFetchOutput, WebSearchInput, WebSearchOutput, WechatArticle, WechatArticleQuality,
    WechatSearchInput, WechatSearchOutput, ZhihuSearchInput, ZhihuSearchItem, ZhihuSearchOutput,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    GithubIssue,
    GithubPull,
    GithubRepo,
    Stackexchange,
    Arxiv,
    Wikipedia,
    AcademicMetadata,
    AcademicPdf,
    Generic,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::GithubIssue => "github_issue",
            SourceType::GithubPull => "github_pull",
            SourceType::GithubRepo => "github_repo",
            SourceType::Stackexchange => "stackexchange",
            SourceType::Arxiv => "arxiv",
            SourceType::Wikipedia => "wikipedia",
            SourceType::AcademicMetadata => "academic_metadata",
            SourceType::AcademicPdf => "academic_pdf",
            SourceType::Generic => "generic",
        }
    }
}
