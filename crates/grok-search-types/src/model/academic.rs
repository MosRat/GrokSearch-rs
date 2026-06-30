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
    pub save_raw_content_path: Option<String>,
    pub images_dir: Option<String>,
    pub tables_dir: Option<String>,
    pub extract_images: Option<bool>,
    pub extract_tables: Option<bool>,
    pub extract_material_links: Option<bool>,
    pub text_processing_mode: Option<String>,
    pub include_raw_content: Option<bool>,
    pub llm_progressive: Option<AcademicLlmProgressiveOptions>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfLocator {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
}

impl AcademicPdfLocator {
    pub fn selected_count(&self) -> usize {
        [
            self.identifier.as_deref(),
            self.url.as_deref(),
            self.pdf_url.as_deref(),
        ]
        .into_iter()
        .filter(|value| value.is_some_and(|value| !value.trim().is_empty()))
        .count()
    }

    pub fn is_valid_exactly_one(&self) -> bool {
        self.selected_count() == 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicLlmProgressiveOptions {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub max_chunk_chars: Option<usize>,
    pub overlap_chars: Option<usize>,
    pub concurrency: Option<usize>,
    pub max_output_tokens: Option<u32>,
    pub input_profile: Option<String>,
    pub prompt_profile: Option<String>,
    pub cache_enabled: Option<bool>,
    pub cache_refresh: Option<bool>,
    pub save_json_path: Option<String>,
    pub include_section_text: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcademicPdfStructureProfile {
    Fast,
    #[default]
    Balanced,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcademicPdfCachePolicy {
    #[default]
    Auto,
    Refresh,
    Bypass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfReadInput {
    #[serde(flatten)]
    pub locator: AcademicPdfLocator,
    pub text_mode: Option<String>,
    pub max_chars: Option<usize>,
    pub include_raw_content: Option<bool>,
    pub include_processing: Option<bool>,
    pub extract_material_links: Option<bool>,
    pub cache_policy: Option<AcademicPdfCachePolicy>,
}

pub type AcademicPdfReadOutput = AcademicReadOutput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfStructureInput {
    #[serde(flatten)]
    pub locator: AcademicPdfLocator,
    pub view: Option<String>,
    pub section_id: Option<String>,
    pub profile: Option<AcademicPdfStructureProfile>,
    pub model: Option<String>,
    pub cache_policy: Option<AcademicPdfCachePolicy>,
    pub include_section_text: Option<bool>,
    pub save_json_path: Option<String>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicPdfStructureOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub pdf_url: String,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    pub view: String,
    pub progressive_reading: AcademicProgressivePaper,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<AcademicProgressiveSection>,
    pub processing: AcademicPdfProcessingReport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<AcademicParseArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_cache: Option<AcademicPdfCacheInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfArtifactsInput {
    #[serde(flatten)]
    pub locator: AcademicPdfLocator,
    pub images_dir: Option<String>,
    pub tables_dir: Option<String>,
    pub extract_images: Option<bool>,
    pub extract_tables: Option<bool>,
    pub text_mode: Option<String>,
    pub max_chars: Option<usize>,
    pub cache_policy: Option<AcademicPdfCachePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicPdfArtifactsOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub pdf_url: String,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    pub artifacts: Vec<AcademicParseArtifact>,
    pub parse_capabilities: AcademicParseCapabilities,
    pub processing: AcademicPdfProcessingReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_cache: Option<AcademicPdfCacheInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfDownloadInput {
    #[serde(flatten)]
    pub locator: AcademicPdfLocator,
    pub output_path: String,
    pub overwrite: Option<bool>,
    pub cache_policy: Option<AcademicPdfCachePolicy>,
}

pub type AcademicPdfDownloadOutput = AcademicDownloadPdfOutput;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveGetInput {
    pub cache_key: String,
    pub view: Option<String>,
    pub section_id: Option<String>,
    pub include_section_text: Option<bool>,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveGetOutput {
    pub cache_key: String,
    pub view: String,
    pub cache_hit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progressive_reading: Option<AcademicProgressivePaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<AcademicProgressiveSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressivePaper {
    pub metadata: AcademicProgressiveMetadata,
    pub budget: AcademicProgressiveBudget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<AcademicProgressiveOutlineNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<AcademicProgressiveSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<AcademicProgressiveFigure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<AcademicProgressiveTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<AcademicProgressiveReference>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub evidence_index: BTreeMap<String, AcademicProgressiveEvidenceSpan>,
    pub llm_report: AcademicProgressiveLlmReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<AcademicProgressiveCacheInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub identifiers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveBudget {
    pub total_chars: usize,
    pub estimated_tokens: usize,
    pub chunk_count: usize,
    pub section_count: usize,
    pub figure_count: usize,
    pub table_count: usize,
    pub reference_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveOutlineNode {
    pub section_id: String,
    pub title: String,
    pub level: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveSection {
    pub section_id: String,
    pub title: String,
    pub level: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_points: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clean_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveEvidenceSpan {
    pub anchor_id: String,
    pub page: Option<usize>,
    pub line_start: usize,
    pub line_end: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveFigure {
    pub figure_id: String,
    pub label: String,
    pub caption: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveTable {
    pub table_id: String,
    pub label: String,
    pub caption: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveReference {
    pub reference_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<AcademicProgressiveEvidenceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveLlmReport {
    pub model: String,
    pub provider: String,
    pub input_profile: String,
    pub prompt_profile: String,
    pub chunk_strategy: String,
    pub max_chunk_chars: usize,
    pub overlap_chars: usize,
    pub max_output_tokens: u32,
    pub concurrency: usize,
    pub calls: usize,
    pub retries: usize,
    pub invalid_json: usize,
    pub fallback_chunks: usize,
    pub accepted_patches: usize,
    pub rejected_patches: usize,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<AcademicProgressiveChunkReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveChunkReport {
    pub chunk_id: String,
    pub start_char: usize,
    pub end_char: usize,
    pub input_chars: usize,
    pub output_chars: usize,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "is_default_u32")]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "is_default_u64")]
    pub backoff_ms: u64,
    pub json_valid: bool,
    pub repaired: bool,
    pub fallback: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn is_default_u32(value: &u32) -> bool {
    *value == 0
}

fn is_default_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicProgressiveCacheInfo {
    pub key: String,
    pub hit: bool,
    pub stored: bool,
    pub strategy_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcademicPdfCacheInfo {
    pub key: String,
    pub hit: bool,
    pub stored: bool,
    pub bytes: u64,
    pub attempts: u32,
    pub backoff_ms: u64,
    pub download_elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
pub struct AcademicPdfProcessingReport {
    pub text_processing_mode: String,
    pub raw_original_length: usize,
    pub processed_original_length: usize,
    pub raw_truncated: bool,
    pub processed_truncated: bool,
    pub passes: Vec<AcademicPdfPassReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicPdfPassReport {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_original_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_truncated: Option<bool>,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<AcademicParseArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_capabilities: Option<AcademicParseCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing: Option<AcademicPdfProcessingReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<AcademicMaterialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progressive_reading: Option<AcademicProgressivePaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_cache: Option<AcademicPdfCacheInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicParsePdfOutput {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: String,
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_original_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_truncated: Option<bool>,
    pub source: String,
    pub fulltext_status: String,
    pub resolver_chain: Vec<String>,
    pub artifacts: Vec<AcademicParseArtifact>,
    pub parse_capabilities: AcademicParseCapabilities,
    pub processing: AcademicPdfProcessingReport,
    pub materials: Vec<AcademicMaterialLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progressive_reading: Option<AcademicProgressivePaper>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_cache: Option<AcademicPdfCacheInfo>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_cache: Option<AcademicPdfCacheInfo>,
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
            raw_content: None,
            raw_original_length: None,
            raw_truncated: None,
            source: "test".to_string(),
            fulltext_status: "ok".to_string(),
            resolver_chain: vec!["test".to_string()],
            artifacts: Vec::new(),
            parse_capabilities: None,
            processing: None,
            materials: Vec::new(),
            progressive_reading: None,
            pdf_cache: None,
        };

        let value = serde_json::to_value(output).expect("serialize read output");
        assert_eq!(value["content"], json!("paper"));
        assert!(value.get("artifacts").is_none());
        assert!(value.get("parse_capabilities").is_none());
        assert!(value.get("processing").is_none());
        assert!(value.get("raw_content").is_none());
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
    fn pdf_locator_requires_exactly_one_location() {
        assert!(AcademicPdfLocator {
            identifier: Some("arXiv:1706.03762".to_string()),
            ..Default::default()
        }
        .is_valid_exactly_one());
        assert!(!AcademicPdfLocator::default().is_valid_exactly_one());
        assert!(!AcademicPdfLocator {
            identifier: Some("arXiv:1706.03762".to_string()),
            url: Some("https://example.com".to_string()),
            pdf_url: None,
        }
        .is_valid_exactly_one());
    }

    #[test]
    fn pdf_profile_and_cache_policy_use_snake_case() {
        assert_eq!(
            serde_json::to_value(AcademicPdfStructureProfile::Balanced).unwrap(),
            json!("balanced")
        );
        assert_eq!(
            serde_json::from_value::<AcademicPdfCachePolicy>(json!("refresh")).unwrap(),
            AcademicPdfCachePolicy::Refresh
        );
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
            pdf_cache: None,
        };

        let value = serde_json::to_value(output).expect("serialize download output");
        assert_eq!(value["identifier"], json!("arXiv:1706.03762"));
        assert_eq!(value["pdf_url"], json!("https://arxiv.org/pdf/1706.03762"));
        assert_eq!(value["path"], json!("papers/attention.pdf"));
        assert_eq!(value["bytes"], json!(1234));
        assert!(value.get("pdf_cache").is_none());
    }
}
