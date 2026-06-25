use std::collections::HashMap;

use grok_search_provider_core::AcademicIdentifier;
use grok_search_types::AcademicPaper;
use serde_json::Value;
use url::Url;

pub fn normalize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
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
    if value.starts_with("10.") || value.to_ascii_lowercase().starts_with("doi:10.") {
        return AcademicIdentifier::Doi(value.trim_start_matches("doi:").to_string());
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
    if value.starts_with("arXiv:") || looks_like_arxiv_id(value) {
        return AcademicIdentifier::Arxiv(value.trim_start_matches("arXiv:").to_string());
    }
    if value.starts_with('W') && value[1..].chars().all(|c| c.is_ascii_digit()) {
        return AcademicIdentifier::OpenAlex(value.to_string());
    }
    if value.len() >= 32 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return AcademicIdentifier::Semantic(value.to_string());
    }
    AcademicIdentifier::Query(value.to_string())
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
    if let Some(doi) = &paper.doi {
        return format!("doi:{}", doi.to_ascii_lowercase());
    }
    if let Some(arxiv) = &paper.arxiv_id {
        return format!("arxiv:{}", arxiv.to_ascii_lowercase());
    }
    if let Some(id) = &paper.semantic_scholar_id {
        return format!("semantic:{}", id.to_ascii_lowercase());
    }
    if let Some(id) = &paper.openalex_id {
        return format!("openalex:{}", id.to_ascii_lowercase());
    }
    format!(
        "title:{}:{}",
        normalize_title(&paper.title),
        paper.year.map(|y| y.to_string()).unwrap_or_default()
    )
}

pub fn rrf_merge_papers(ranked: Vec<(String, Vec<AcademicPaper>)>) -> Vec<AcademicPaper> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut papers: HashMap<String, AcademicPaper> = HashMap::new();
    for (_source, list) in ranked {
        for (idx, paper) in list.into_iter().enumerate() {
            let key = academic_paper_key(&paper);
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
            parse_academic_identifier("10.1145/3368089.3409742"),
            AcademicIdentifier::Doi("10.1145/3368089.3409742".to_string())
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
}
