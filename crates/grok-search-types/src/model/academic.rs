use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::source::Source;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicSearchInput {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    pub search_mode: Option<String>,
    pub sort_by: Option<String>,
    pub max_results: Option<usize>,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub open_access_only: Option<bool>,
    pub include_abstract: Option<bool>,
    pub include_citations: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicGetInput {
    pub identifier: String,
    pub include_citations: Option<bool>,
    pub include_open_access: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicCitationsInput {
    pub identifier: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicReadInput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub max_chars: Option<usize>,
    pub output_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPaper {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub venue: Option<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub semantic_scholar_id: Option<String>,
    pub openalex_id: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub citation_count: Option<u32>,
    pub reference_count: Option<u32>,
    pub open_access: Option<bool>,
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
}

impl AcademicPaper {
    pub fn merge_from(&mut self, other: AcademicPaper) {
        if self.title.is_empty() {
            self.title = other.title;
        }
        if self.authors.is_empty() {
            self.authors = other.authors;
        }
        self.year = self.year.or(other.year);
        self.venue = self.venue.take().or(other.venue);
        self.abstract_text = self.abstract_text.take().or(other.abstract_text);
        self.doi = self.doi.take().or(other.doi);
        self.arxiv_id = self.arxiv_id.take().or(other.arxiv_id);
        self.semantic_scholar_id = self
            .semantic_scholar_id
            .take()
            .or(other.semantic_scholar_id);
        self.openalex_id = self.openalex_id.take().or(other.openalex_id);
        self.url = self.url.take().or(other.url);
        self.pdf_url = self.pdf_url.take().or(other.pdf_url);
        self.citation_count = self.citation_count.max(other.citation_count);
        self.reference_count = self.reference_count.max(other.reference_count);
        self.open_access = self.open_access.or(other.open_access);
        self.license = self.license.take().or(other.license);
        for source in other.sources {
            if !self
                .sources
                .iter()
                .any(|existing| existing.url == source.url)
            {
                self.sources.push(source);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicSearchOutput {
    pub session_id: String,
    pub papers_count: usize,
    pub papers: Vec<AcademicPaper>,
    pub sources_used: Vec<String>,
    pub errors: BTreeMap<String, String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicGetOutput {
    pub paper: AcademicPaper,
    pub citations: Option<AcademicCitationSummary>,
    pub resolver_chain: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicCitationSummary {
    #[serde(default)]
    pub citations: Vec<AcademicPaper>,
    #[serde(default)]
    pub references: Vec<AcademicPaper>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicCitationsOutput {
    pub identifier: String,
    pub citation_count: Option<u32>,
    pub reference_count: Option<u32>,
    pub citations: Vec<AcademicPaper>,
    pub references: Vec<AcademicPaper>,
    pub sources_used: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicReadOutput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: String,
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
}
