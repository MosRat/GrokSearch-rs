use grok_search_types::{AcademicMaterialLink, AcademicPaper};

pub(super) fn material_links_for_paper(paper: &AcademicPaper) -> Vec<AcademicMaterialLink> {
    let mut materials = Vec::new();
    for (url, source) in [
        (paper.url.as_deref(), "paper_url"),
        (paper.pdf_url.as_deref(), "paper_pdf_url"),
    ] {
        if let Some(url) = url {
            materials.extend(material_links_from_url(url, source));
        }
    }
    if let Some(abstract_text) = &paper.abstract_text {
        materials.extend(material_links_from_text(abstract_text, "abstract"));
    }
    for source in &paper.sources {
        materials.extend(material_links_from_url(
            &source.url,
            source.provider.as_ref(),
        ));
    }
    merge_materials(Vec::new(), materials)
}

pub(super) fn material_links_from_text(text: &str, source: &str) -> Vec<AcademicMaterialLink> {
    text.split_whitespace()
        .filter_map(|token| {
            let url = token
                .trim_matches(|ch: char| {
                    matches!(
                        ch,
                        '"' | '\''
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | ','
                            | '.'
                            | ';'
                    )
                })
                .trim_end_matches('/');
            material_links_from_url(url, source).into_iter().next()
        })
        .collect()
}

pub(super) fn material_links_from_url(url: &str, source: &str) -> Vec<AcademicMaterialLink> {
    let Some(kind) = classify_material_url(url) else {
        return Vec::new();
    };
    vec![AcademicMaterialLink {
        url: url.to_string(),
        kind: kind.to_string(),
        source: source.to_string(),
        confidence: material_confidence(kind).to_string(),
        title: None,
    }]
}

fn classify_material_url(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    if lower.contains("github.com/") || lower.contains("gitlab.com/") {
        return Some("code");
    }
    if lower.contains("huggingface.co/") {
        if lower.contains("/datasets/") {
            return Some("dataset");
        }
        if lower.contains("/spaces/") {
            return Some("demo");
        }
        return Some("model");
    }
    if lower.contains("paperswithcode.com/") {
        return Some("code");
    }
    if lower.contains("zenodo.org/") || lower.contains("figshare.com/") {
        return Some("dataset");
    }
    if lower.contains("arxiv.org/src/") || lower.contains("arxiv.org/e-print/") {
        return Some("supplement");
    }
    if lower.contains("colab.research.google.com/") {
        return Some("demo");
    }
    if lower.contains("docs.") || lower.contains("/docs") || lower.contains("readthedocs.io/") {
        return Some("documentation");
    }
    if lower.contains("project")
        || lower.contains("demo")
        || lower.contains("dataset")
        || lower.contains("code")
    {
        return Some("project");
    }
    None
}

fn material_confidence(kind: &str) -> &'static str {
    match kind {
        "code" | "dataset" | "model" | "demo" | "supplement" => "high",
        "documentation" => "medium",
        _ => "low",
    }
}

pub(super) fn merge_materials(
    first: Vec<AcademicMaterialLink>,
    second: Vec<AcademicMaterialLink>,
) -> Vec<AcademicMaterialLink> {
    let mut out = Vec::new();
    for material in first.into_iter().chain(second) {
        if !out
            .iter()
            .any(|existing: &AcademicMaterialLink| existing.url.eq_ignore_ascii_case(&material.url))
        {
            out.push(material);
        }
    }
    out
}
