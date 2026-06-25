pub mod error;
pub mod model;

pub use error::{GrokSearchError, Result};
pub use model::academic::{
    AcademicCitationSummary, AcademicCitationsInput, AcademicCitationsOutput, AcademicGetInput,
    AcademicGetOutput, AcademicPaper, AcademicReadInput, AcademicReadOutput, AcademicSearchInput,
    AcademicSearchOutput,
};
pub use model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchResponse, SearchTool,
};
pub use model::source::{merge_sources, Source};
pub use model::tool::{GetSourcesOutput, WebFetchOutput, WebSearchInput, WebSearchOutput};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    GithubIssue,
    GithubPull,
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
            SourceType::Stackexchange => "stackexchange",
            SourceType::Arxiv => "arxiv",
            SourceType::Wikipedia => "wikipedia",
            SourceType::AcademicMetadata => "academic_metadata",
            SourceType::AcademicPdf => "academic_pdf",
            SourceType::Generic => "generic",
        }
    }
}
