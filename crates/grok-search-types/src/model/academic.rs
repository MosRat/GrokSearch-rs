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
    pub extract_material_links: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicGetInput {
    pub identifier: String,
    pub include_citations: Option<bool>,
    pub include_open_access: Option<bool>,
    pub extract_material_links: Option<bool>,
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
    pub parse_options: Option<AcademicParseOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicParseOptions {
    pub save_markdown_path: Option<String>,
    pub images_dir: Option<String>,
    pub tables_dir: Option<String>,
    pub extract_images: Option<bool>,
    pub extract_tables: Option<bool>,
    pub extract_material_links: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicParsePdfInput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub max_chars: Option<usize>,
    pub output_format: Option<String>,
    pub parse_options: Option<AcademicParseOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicDownloadPdfInput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub output_path: String,
    pub overwrite: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicParseArtifact {
    pub kind: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicParseCapabilities {
    pub markdown: String,
    pub text: String,
    pub images: String,
    pub tables: String,
    pub material_links: String,
}

impl Default for AcademicParseCapabilities {
    fn default() -> Self {
        Self {
            markdown: "supported".to_string(),
            text: "supported".to_string(),
            images: "partial".to_string(),
            tables: "partial".to_string(),
            material_links: "supported".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicMaterialLink {
    pub url: String,
    pub kind: String,
    pub source: String,
    pub confidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<AcademicMaterialLink>,
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
        for material in other.materials {
            if !self
                .materials
                .iter()
                .any(|existing| existing.url == material.url)
            {
                self.materials.push(material);
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<AcademicParseArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_capabilities: Option<AcademicParseCapabilities>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<AcademicMaterialLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicParsePdfOutput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: String,
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    pub artifacts: Vec<AcademicParseArtifact>,
    pub parse_capabilities: AcademicParseCapabilities,
    pub materials: Vec<AcademicMaterialLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicDownloadPdfOutput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: String,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    pub path: String,
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn academic_read_output_skips_empty_parse_extensions() {
        let output = AcademicReadOutput {
            identifier: Some("10.123/example".to_string()),
            url: None,
            pdf_url: "https://example.com/paper.pdf".to_string(),
            content: "paper".to_string(),
            original_length: 5,
            truncated: false,
            source: "test".to_string(),
            fulltext_status: "ok".to_string(),
            resolver_chain: vec!["test".to_string()],
            artifacts: Vec::new(),
            parse_capabilities: None,
            materials: Vec::new(),
        };

        let value = serde_json::to_value(output).expect("serialize read output");
        assert_eq!(value["content"], json!("paper"));
        assert!(value.get("artifacts").is_none());
        assert!(value.get("parse_capabilities").is_none());
        assert!(value.get("materials").is_none());
    }

    #[test]
    fn academic_parse_capabilities_default_is_stable() {
        let capabilities = AcademicParseCapabilities::default();
        assert_eq!(capabilities.markdown, "supported");
        assert_eq!(capabilities.text, "supported");
        assert_eq!(capabilities.images, "partial");
        assert_eq!(capabilities.tables, "partial");
        assert_eq!(capabilities.material_links, "supported");
    }

    #[test]
    fn academic_download_pdf_output_shape_is_stable() {
        let output = AcademicDownloadPdfOutput {
            identifier: Some("arXiv:1706.03762".to_string()),
            url: None,
            pdf_url: "https://arxiv.org/pdf/1706.03762".to_string(),
            source: "arxiv".to_string(),
            fulltext_status: "resolved".to_string(),
            resolver_chain: vec!["arxiv".to_string()],
            path: "papers/attention.pdf".to_string(),
            bytes: 1234,
        };

        let value = serde_json::to_value(output).expect("serialize download output");
        assert_eq!(value["identifier"], json!("arXiv:1706.03762"));
        assert_eq!(value["pdf_url"], json!("https://arxiv.org/pdf/1706.03762"));
        assert_eq!(value["path"], json!("papers/attention.pdf"));
        assert_eq!(value["bytes"], json!(1234));
    }
}
