use grok_search_parse::normalize_title;
use grok_search_types::AcademicPaper;

use super::search_options::AcademicSortBy;

pub(super) fn paper_matches_year_filter(
    paper: &AcademicPaper,
    year_from: Option<u32>,
    year_to: Option<u32>,
) -> bool {
    let Some(year) = paper.year else {
        return true;
    };
    year_from.map_or(true, |from| year >= from) && year_to.map_or(true, |to| year <= to)
}

pub(super) fn search_result_is_relevant(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {}",
        paper.title,
        paper.abstract_text.as_deref().unwrap_or_default()
    );
    let haystack_tokens = meaningful_tokens(&haystack);
    let matches = matching_query_tokens(&query_tokens, &haystack_tokens);
    matches >= min_required_query_token_matches(query_tokens.len())
}

pub(super) fn search_result_has_strong_overlap(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.len() <= 2 {
        return search_result_is_relevant(query, paper);
    }
    let haystack = format!(
        "{} {}",
        paper.title,
        paper.abstract_text.as_deref().unwrap_or_default()
    );
    let haystack_tokens = meaningful_tokens(&haystack);
    let matches = matching_query_tokens(&query_tokens, &haystack_tokens);
    matches >= strong_required_query_token_matches(query_tokens.len())
}

pub(super) fn precise_search_result_is_relevant(query: &str, paper: &AcademicPaper) -> bool {
    let query_tokens = meaningful_tokens(query);
    if query_tokens.is_empty() {
        return true;
    }
    let title_tokens = meaningful_tokens(&paper.title);
    let title_matches = matching_query_tokens(&query_tokens, &title_tokens);
    title_matches >= min_required_query_token_matches(query_tokens.len())
        || normalize_title(&paper.title).contains(&normalize_title(query))
}

pub(super) fn rank_academic_results(
    query: &str,
    sort_by: AcademicSortBy,
    papers: &mut [AcademicPaper],
) {
    let query_tokens = meaningful_tokens(query);
    papers.sort_by(|a, b| {
        academic_result_score(query, &query_tokens, sort_by, b).cmp(&academic_result_score(
            query,
            &query_tokens,
            sort_by,
            a,
        ))
    });
}

fn academic_result_score(
    query: &str,
    query_tokens: &[String],
    sort_by: AcademicSortBy,
    paper: &AcademicPaper,
) -> u32 {
    let title_tokens = meaningful_tokens(&paper.title);
    let abstract_tokens = meaningful_tokens(paper.abstract_text.as_deref().unwrap_or_default());
    let title_matches = matching_query_tokens(query_tokens, &title_tokens) as u32;
    let abstract_matches = matching_query_tokens(query_tokens, &abstract_tokens) as u32;
    let exact_title = (normalize_title(&paper.title) == normalize_title(query)) as u32;
    let pdf = paper.pdf_url.is_some() as u32;
    let oa = paper.open_access.unwrap_or(false) as u32;
    let citations = paper.citation_count.unwrap_or(0).min(10_000);
    let citation_score = if citations == 0 {
        0
    } else {
        citations.ilog10()
    };
    let citation_preference = match sort_by {
        AcademicSortBy::Citations => citation_score * 40 + citations.min(1_000) / 25,
        _ => citation_score,
    };
    let date_preference = match sort_by {
        AcademicSortBy::Date => paper.year.unwrap_or(0).saturating_sub(1900).min(200),
        _ => 0,
    };

    exact_title * 1_000
        + title_matches * 100
        + abstract_matches * 20
        + citation_preference
        + date_preference
        + pdf * 3
        + oa
}

fn matching_query_tokens(query_tokens: &[String], haystack_tokens: &[String]) -> usize {
    query_tokens
        .iter()
        .filter(|token| haystack_tokens.iter().any(|candidate| candidate == *token))
        .count()
}

fn min_required_query_token_matches(query_token_count: usize) -> usize {
    if query_token_count <= 2 {
        1
    } else {
        2
    }
}

fn strong_required_query_token_matches(query_token_count: usize) -> usize {
    query_token_count.min(3)
}

fn meaningful_tokens(text: &str) -> Vec<String> {
    normalize_title(text)
        .split_whitespace()
        .filter(|token| token.len() >= 3 && !ACADEMIC_STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

const ACADEMIC_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "for", "from", "how", "into", "not", "of", "on", "or", "the", "this",
    "to", "with", "paper",
];
