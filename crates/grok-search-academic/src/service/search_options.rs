use grok_search_types::{GrokSearchError, Result};
use uuid::Uuid;

use super::{ALL_SOURCES, DEFAULT_SOURCES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcademicSearchMode {
    Balanced,
    Broad,
    Precise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AcademicSortBy {
    Relevance,
    Citations,
    Date,
}

pub(super) fn search_mode(raw: Option<&str>) -> Result<AcademicSearchMode> {
    match raw
        .unwrap_or("balanced")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "balanced" => Ok(AcademicSearchMode::Balanced),
        "broad" => Ok(AcademicSearchMode::Broad),
        "precise" => Ok(AcademicSearchMode::Precise),
        other => Err(GrokSearchError::InvalidParams(format!(
            "search_mode must be \"balanced\", \"broad\", or \"precise\", got \"{other}\""
        ))),
    }
}

pub(super) fn academic_sort_by(raw: Option<&str>) -> Result<AcademicSortBy> {
    match raw
        .unwrap_or("relevance")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "relevance" => Ok(AcademicSortBy::Relevance),
        "citations" => Ok(AcademicSortBy::Citations),
        "date" => Ok(AcademicSortBy::Date),
        other => Err(GrokSearchError::InvalidParams(format!(
            "sort_by must be \"relevance\", \"citations\", or \"date\", got \"{other}\""
        ))),
    }
}

pub(super) fn selected_sources(raw: &[String], mode: AcademicSearchMode) -> Result<Vec<String>> {
    let requested: Vec<String> = if raw.is_empty() {
        let defaults = match mode {
            AcademicSearchMode::Balanced | AcademicSearchMode::Precise => DEFAULT_SOURCES,
            AcademicSearchMode::Broad => ALL_SOURCES,
        };
        defaults.iter().map(|s| s.to_string()).collect()
    } else {
        raw.iter()
            .flat_map(|s| s.split(','))
            .map(canonical_academic_source)
            .filter(|s| !s.is_empty())
            .collect()
    };
    let unknown = requested
        .iter()
        .find(|source| !ALL_SOURCES.contains(&source.as_str()));
    if let Some(source) = unknown {
        return Err(GrokSearchError::InvalidParams(format!(
            "unknown academic source: {source}; expected one of {}",
            ALL_SOURCES.join(", ")
        )));
    }
    Ok(requested)
}

fn canonical_academic_source(source: &str) -> String {
    match source.trim().to_ascii_lowercase().as_str() {
        "semantic_scholar" | "semanticscholar" | "s2" => "semantic".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn short_session_id() -> String {
    let mut uuid_buf = [0u8; uuid::fmt::Simple::LENGTH];
    Uuid::new_v4().simple().encode_lower(&mut uuid_buf)[..12].to_string()
}
