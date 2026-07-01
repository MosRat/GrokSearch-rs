use std::cmp::Reverse;

use grok_search_parse::normalize_title;
use grok_search_provider_core::AcademicIdentifier as Identifier;
use grok_search_types::{AcademicCitationSummary, AcademicPaper};

use crate::providers::without_openalex_reference_sources;

pub(super) fn identifier_for_paper(paper: &AcademicPaper) -> Identifier {
    paper
        .doi
        .as_ref()
        .map(|v| Identifier::Doi(v.clone()))
        .or_else(|| {
            paper
                .arxiv_id
                .as_ref()
                .map(|v| Identifier::Arxiv(v.clone()))
        })
        .or_else(|| {
            paper
                .semantic_scholar_id
                .as_ref()
                .map(|v| Identifier::Semantic(v.clone()))
        })
        .or_else(|| {
            paper
                .openalex_id
                .as_ref()
                .map(|v| Identifier::OpenAlex(v.clone()))
        })
        .unwrap_or_else(|| Identifier::Query(paper.title.clone()))
}

pub(super) fn citation_identifiers_for_paper(paper: &AcademicPaper) -> Vec<Identifier> {
    let mut ids = Vec::new();
    if let Some(id) = &paper.semantic_scholar_id {
        ids.push(Identifier::Semantic(id.clone()));
    }
    if let Some(id) = &paper.openalex_id {
        ids.push(Identifier::OpenAlex(id.clone()));
    }
    if let Some(doi) = &paper.doi {
        ids.push(Identifier::Doi(doi.clone()));
    }
    if let Some(id) = &paper.arxiv_id {
        ids.push(Identifier::Arxiv(id.clone()));
    }
    if ids.is_empty() {
        ids.push(Identifier::Query(paper.title.clone()));
    }
    ids
}

pub(super) fn resolved_paper_matches_identifier(id: &Identifier, paper: &AcademicPaper) -> bool {
    match id {
        Identifier::Query(query) => normalize_title(&paper.title) == normalize_title(query),
        _ => true,
    }
}

pub(super) fn select_best_title_match(
    query: &str,
    papers: impl IntoIterator<Item = AcademicPaper>,
) -> Option<AcademicPaper> {
    let expected = normalize_title(query);
    let mut matches: Vec<AcademicPaper> = papers
        .into_iter()
        .filter(|paper| normalize_title(&paper.title) == expected)
        .collect();
    matches.sort_by_key(|paper| Reverse(canonical_title_score(query, paper)));
    matches.into_iter().next()
}

pub(super) fn merge_canonical_candidates(mut candidates: Vec<AcademicPaper>) -> AcademicPaper {
    candidates.sort_by_key(|paper| Reverse(canonical_identifier_score(paper)));
    let mut merged = candidates.remove(0);
    for candidate in candidates {
        merged.merge_from(candidate);
    }
    merged
}

fn canonical_title_score(query: &str, paper: &AcademicPaper) -> u32 {
    let exact_title = (normalize_title(&paper.title) == normalize_title(query)) as u32;
    exact_title * 10_000
        + canonical_identifier_score(paper)
        + canonical_source_score(paper)
        + author_signal_score(paper)
        + venue_signal_score(paper)
        + stable_year_score(paper)
        + citation_signal_score(paper)
        + suspicious_doi_penalty(paper)
}

fn canonical_identifier_score(paper: &AcademicPaper) -> u32 {
    paper.semantic_scholar_id.is_some() as u32 * 2_000
        + paper.arxiv_id.is_some() as u32 * 1_600
        + paper
            .doi
            .as_ref()
            .map_or(0, |doi| if suspicious_doi(doi, paper) { 100 } else { 700 })
        + paper.openalex_id.is_some() as u32 * 300
}

fn canonical_source_score(paper: &AcademicPaper) -> u32 {
    paper
        .sources
        .iter()
        .map(|source| match source.provider.as_ref() {
            "semantic" => 900,
            "arxiv" => 800,
            "dblp" => 700,
            "openalex" => 250,
            "crossref" => 150,
            _ => 0,
        })
        .sum::<u32>()
        .min(2_400)
}

fn author_signal_score(paper: &AcademicPaper) -> u32 {
    (paper.authors.len().min(8) as u32) * 25
}

fn venue_signal_score(paper: &AcademicPaper) -> u32 {
    match paper.venue.as_deref().map(|v| v.to_ascii_lowercase()) {
        Some(venue)
            if venue.contains("arxiv")
                || venue.contains("neural information processing")
                || venue.contains("conference")
                || venue.contains("journal") =>
        {
            250
        }
        Some(_) => 120,
        None => 0,
    }
}

fn stable_year_score(paper: &AcademicPaper) -> u32 {
    match paper.year {
        Some(1900..=2026) => 200,
        Some(_) => 0,
        None => 50,
    }
}

fn citation_signal_score(paper: &AcademicPaper) -> u32 {
    let citations = paper.citation_count.unwrap_or(0).min(100_000);
    if citations == 0 {
        0
    } else {
        citations.ilog10() * 120 + citations.min(10_000) / 20
    }
}

fn suspicious_doi_penalty(paper: &AcademicPaper) -> u32 {
    paper
        .doi
        .as_ref()
        .filter(|doi| suspicious_doi(doi, paper))
        .map_or(0, |_| 0)
}

fn suspicious_doi(doi: &str, paper: &AcademicPaper) -> bool {
    let doi = doi.to_ascii_lowercase();
    let source_only_crossref_or_openalex = !paper.sources.is_empty()
        && paper.sources.iter().all(|source| {
            matches!(
                source.provider.as_ref(),
                "crossref" | "openalex" | "openalex_reference"
            )
        });
    doi.contains("10.65215")
        || (source_only_crossref_or_openalex
            && paper.semantic_scholar_id.is_none()
            && paper.arxiv_id.is_none()
            && paper.venue.is_none())
}

pub(super) fn clean_citation_summary(
    mut summary: AcademicCitationSummary,
) -> AcademicCitationSummary {
    summary.citations = summary
        .citations
        .into_iter()
        .map(without_openalex_reference_sources)
        .collect();
    summary.references = summary
        .references
        .into_iter()
        .map(without_openalex_reference_sources)
        .collect();
    summary
}
