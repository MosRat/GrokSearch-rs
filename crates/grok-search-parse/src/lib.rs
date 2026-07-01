use std::collections::HashMap;

use grok_search_provider_core::AcademicIdentifier;
use grok_search_types::AcademicPaper;
use serde_json::Value;
use url::Url;

pub fn normalize_title(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub fn clean_html_title(title: &str) -> String {
    title
        .replace("<sub>", "")
        .replace("</sub>", "")
        .replace("<sup>", "")
        .replace("</sup>", "")
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn parse_academic_identifier(raw: &str) -> AcademicIdentifier {
    let value = raw.trim();
    let lower = value.to_ascii_lowercase();
    let value_without_doi_prefix = lower
        .strip_prefix("doi:")
        .and_then(|_| value.get(4..))
        .unwrap_or(value)
        .trim();
    if let Some(arxiv_id) = arxiv_id_from_doi(value_without_doi_prefix) {
        return AcademicIdentifier::Arxiv(arxiv_id);
    }
    if value_without_doi_prefix.starts_with("10.") {
        return AcademicIdentifier::Doi(value_without_doi_prefix.to_string());
    }
    if let Ok(url) = Url::parse(value) {
        let host = url.host_str().unwrap_or_default();
        if host.ends_with("arxiv.org") {
            if let Some(id) = extract_arxiv_id_from_path(url.path()) {
                return AcademicIdentifier::Arxiv(id);
            }
        }
        if host.ends_with("openalex.org") {
            return AcademicIdentifier::OpenAlex(value.to_string());
        }
        if host.ends_with("dblp.org") {
            return AcademicIdentifier::Dblp(value.to_string());
        }
        return AcademicIdentifier::Url(value.to_string());
    }
    if lower.starts_with("arxiv:") || looks_like_arxiv_id(value) {
        return AcademicIdentifier::Arxiv(
            value
                .strip_prefix("arXiv:")
                .or_else(|| value.strip_prefix("arxiv:"))
                .unwrap_or(value)
                .to_string(),
        );
    }
    if value.starts_with('W') && value[1..].chars().all(|c| c.is_ascii_digit()) {
        return AcademicIdentifier::OpenAlex(value.to_string());
    }
    if value.len() >= 32 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return AcademicIdentifier::Semantic(value.to_string());
    }
    AcademicIdentifier::Query(value.to_string())
}

fn arxiv_id_from_doi(value: &str) -> Option<String> {
    value
        .strip_prefix("10.48550/arXiv.")
        .or_else(|| value.strip_prefix("10.48550/arxiv."))
        .map(|id| id.trim().trim_end_matches(".pdf").to_string())
        .filter(|id| looks_like_arxiv_id(id))
}

pub fn extract_arxiv_id_from_path(path: &str) -> Option<String> {
    for prefix in ["/abs/", "/pdf/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return Some(rest.strip_suffix(".pdf").unwrap_or(rest).to_string());
        }
    }
    None
}

pub fn looks_like_arxiv_id(value: &str) -> bool {
    let value = value.strip_suffix(".pdf").unwrap_or(value);
    let mut parts = value.split('.');
    matches!((parts.next(), parts.next()), (Some(a), Some(b)) if a.len() == 4 && b.len() >= 4 && a.chars().all(|c| c.is_ascii_digit()))
}

pub fn academic_paper_key(paper: &AcademicPaper) -> String {
    strong_academic_paper_key(paper).unwrap_or_else(|| title_year_key(paper))
}

pub fn rrf_merge_papers(ranked: Vec<(String, Vec<AcademicPaper>)>) -> Vec<AcademicPaper> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut papers: HashMap<String, AcademicPaper> = HashMap::new();
    for (_source, list) in ranked {
        for (idx, paper) in list.into_iter().enumerate() {
            let key = merge_key_for_paper(&papers, &paper);
            *scores.entry(key.clone()).or_default() += 1.0 / (60.0 + idx as f64 + 1.0);
            papers
                .entry(key)
                .and_modify(|existing| existing.merge_from(paper.clone()))
                .or_insert(paper);
        }
    }
    let mut items: Vec<_> = papers.into_iter().collect();
    items.sort_by(|(a, _), (b, _)| {
        scores
            .get(b)
            .partial_cmp(&scores.get(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.into_iter().map(|(_, paper)| paper).collect()
}

fn merge_key_for_paper(existing: &HashMap<String, AcademicPaper>, paper: &AcademicPaper) -> String {
    let title = normalize_title(&paper.title);
    if !title.is_empty() {
        if let Some((key, _)) = existing.iter().find(|(_, candidate)| {
            normalize_title(&candidate.title) == title
                && compatible_years(candidate.year, paper.year)
        }) {
            return key.clone();
        }
    }
    academic_paper_key(paper)
}

fn strong_academic_paper_key(paper: &AcademicPaper) -> Option<String> {
    if let Some(doi) = &paper.doi {
        return Some(format!("doi:{}", doi.to_ascii_lowercase()));
    }
    if let Some(arxiv) = &paper.arxiv_id {
        return Some(format!("arxiv:{}", arxiv.to_ascii_lowercase()));
    }
    if let Some(id) = &paper.semantic_scholar_id {
        return Some(format!("semantic:{}", id.to_ascii_lowercase()));
    }
    if let Some(id) = &paper.openalex_id {
        return Some(format!("openalex:{}", id.to_ascii_lowercase()));
    }
    None
}

fn title_year_key(paper: &AcademicPaper) -> String {
    format!(
        "title:{}:{}",
        normalize_title(&paper.title),
        paper.year.map(|y| y.to_string()).unwrap_or_default()
    )
}

fn compatible_years(a: Option<u32>, b: Option<u32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= 2,
        _ => true,
    }
}

pub fn openalex_abstract(value: &Value) -> String {
    let Some(map) = value.as_object() else {
        return String::new();
    };
    let mut words: Vec<(usize, &str)> = Vec::new();
    for (word, positions) in map {
        if let Some(items) = positions.as_array() {
            for pos in items {
                if let Some(pos) = pos.as_u64() {
                    words.push((pos as usize, word));
                }
            }
        }
    }
    words.sort_by_key(|(pos, _)| *pos);
    words
        .into_iter()
        .map(|(_, word)| word)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_search_types::Source;
    use serde_json::json;

    #[test]
    fn identifier_normalizes_common_academic_ids() {
        assert_eq!(
            parse_academic_identifier("https://arxiv.org/pdf/1706.03762.pdf"),
            AcademicIdentifier::Arxiv("1706.03762".to_string())
        );
        assert_eq!(
            parse_academic_identifier("10.48550/arXiv.1706.03762"),
            AcademicIdentifier::Arxiv("1706.03762".to_string())
        );
        assert_eq!(
            parse_academic_identifier("doi:10.48550/arXiv.1706.03762v7"),
            AcademicIdentifier::Arxiv("1706.03762v7".to_string())
        );
        assert_eq!(
            parse_academic_identifier("10.1145/3368089.3409742"),
            AcademicIdentifier::Doi("10.1145/3368089.3409742".to_string())
        );
    }

    #[test]
    fn normalize_title_treats_punctuation_as_token_boundaries() {
        assert_eq!(
            normalize_title("Retrieval-Augmented Generation (RAG)"),
            "retrieval augmented generation rag"
        );
    }

    #[test]
    fn openalex_inverted_abstract_is_reconstructed() {
        assert_eq!(
            openalex_abstract(&json!({ "hello": [0], "world": [1] })),
            "hello world"
        );
    }

    #[test]
    fn rrf_merge_dedupes_by_doi_and_keeps_sources() {
        let a = AcademicPaper {
            id: "a".into(),
            title: "Same".into(),
            doi: Some("10.1/same".into()),
            sources: vec![Source::new("https://dblp.org/x", "dblp")],
            ..Default::default()
        };
        let b = AcademicPaper {
            id: "b".into(),
            title: "Same".into(),
            doi: Some("10.1/same".into()),
            citation_count: Some(10),
            sources: vec![Source::new("https://semanticscholar.org/x", "semantic")],
            ..Default::default()
        };
        let merged = rrf_merge_papers(vec![("dblp".into(), vec![a]), ("semantic".into(), vec![b])]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].citation_count, Some(10));
        assert_eq!(merged[0].sources.len(), 2);
    }

    #[test]
    fn rrf_merge_dedupes_same_title_when_years_are_close() {
        let a = AcademicPaper {
            id: "a".into(),
            title: "Attention Is All You Need".into(),
            year: Some(2017),
            sources: vec![Source::new("https://semantic.example/a", "semantic")],
            ..Default::default()
        };
        let b = AcademicPaper {
            id: "b".into(),
            title: "Attention is all you need".into(),
            year: Some(2018),
            citation_count: Some(100),
            sources: vec![Source::new("https://arxiv.example/b", "arxiv")],
            ..Default::default()
        };
        let merged = rrf_merge_papers(vec![
            ("semantic".into(), vec![a]),
            ("arxiv".into(), vec![b]),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].citation_count, Some(100));
        assert_eq!(merged[0].sources.len(), 2);
    }

    #[test]
    fn rrf_merge_keeps_same_title_apart_when_years_are_far_without_shared_ids() {
        let a = AcademicPaper {
            id: "a".into(),
            title: "A Reused Paper Title".into(),
            year: Some(2017),
            ..Default::default()
        };
        let b = AcademicPaper {
            id: "b".into(),
            title: "A Reused Paper Title".into(),
            year: Some(2025),
            ..Default::default()
        };
        let merged = rrf_merge_papers(vec![("a".into(), vec![a]), ("b".into(), vec![b])]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn rrf_merge_dedupes_far_years_when_strong_id_matches() {
        let a = AcademicPaper {
            id: "a".into(),
            title: "A Preprint Title".into(),
            arxiv_id: Some("2401.00001".into()),
            year: Some(2017),
            ..Default::default()
        };
        let b = AcademicPaper {
            id: "b".into(),
            title: "A Preprint Title".into(),
            arxiv_id: Some("2401.00001".into()),
            year: Some(2025),
            citation_count: Some(3),
            ..Default::default()
        };
        let merged = rrf_merge_papers(vec![("a".into(), vec![a]), ("b".into(), vec![b])]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].citation_count, Some(3));
    }
}
