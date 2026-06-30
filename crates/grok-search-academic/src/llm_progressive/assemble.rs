use std::collections::{BTreeMap, HashMap, HashSet};

use grok_search_pdf::PdfProgressiveSourceBundle;
use grok_search_types::{
    AcademicProgressiveBudget, AcademicProgressiveEvidenceSpan, AcademicProgressiveFigure,
    AcademicProgressiveLlmReport, AcademicProgressiveMetadata, AcademicProgressiveOutlineNode,
    AcademicProgressivePaper, AcademicProgressiveReference, AcademicProgressiveSection,
    AcademicProgressiveTable,
};

use super::chunking::Chunk;
use super::schema::{ChunkResult, Entity, SectionCandidate};
use super::ProgressiveRunConfig;

pub(crate) fn assemble_paper(
    text: &str,
    source: Option<&PdfProgressiveSourceBundle>,
    chunks: &[Chunk],
    results: &[ChunkResult],
    run_config: &ProgressiveRunConfig,
    elapsed_ms: u64,
) -> AcademicProgressivePaper {
    let evidence_index = chunks
        .iter()
        .flat_map(|chunk| chunk.anchors.iter().cloned())
        .map(|anchor| (anchor.anchor_id.clone(), anchor))
        .collect::<BTreeMap<_, _>>();
    let candidate_sections = credible_sections(results, &evidence_index);
    let mut sections = sections_from_candidates(text, chunks, &candidate_sections, &evidence_index);
    if sections.is_empty() {
        sections.push(fallback_section(text, chunks, &evidence_index));
    }
    attach_chunk_digests(&mut sections, results);
    let (figures, tables, mut references) = entities_from_results(results, &evidence_index);
    references.extend(reference_tail_items(
        source,
        &evidence_index,
        references.len(),
    ));
    let outline = sections
        .iter()
        .map(|section| AcademicProgressiveOutlineNode {
            section_id: section.section_id.clone(),
            title: section.title.clone(),
            level: section.level,
            source_spans: section.source_spans.clone(),
        })
        .collect::<Vec<_>>();
    let warnings = source
        .map(|source| source.warnings.clone())
        .unwrap_or_default();
    let llm_report = AcademicProgressiveLlmReport {
        model: run_config.model.clone(),
        provider: "anthropic_compatible".to_string(),
        input_profile: run_config.input_profile.clone(),
        prompt_profile: run_config.prompt_profile.clone(),
        chunk_strategy: "paragraph_window".to_string(),
        max_chunk_chars: run_config.max_chunk_chars,
        overlap_chars: run_config.overlap_chars,
        max_output_tokens: run_config.max_output_tokens,
        concurrency: run_config.concurrency,
        calls: results
            .iter()
            .filter(|result| !result.report.fallback)
            .count(),
        retries: results
            .iter()
            .filter(|result| result.report.repaired)
            .count(),
        invalid_json: results
            .iter()
            .filter(|result| !result.report.json_valid)
            .count(),
        fallback_chunks: results
            .iter()
            .filter(|result| result.report.fallback)
            .count(),
        accepted_patches: accepted_patch_count(text, chunks, results),
        rejected_patches: rejected_patch_count(text, chunks, results),
        elapsed_ms,
        chunks: results.iter().map(|result| result.report.clone()).collect(),
        warnings,
    };
    AcademicProgressivePaper {
        metadata: infer_metadata(text, &evidence_index),
        budget: AcademicProgressiveBudget {
            total_chars: text.chars().count(),
            estimated_tokens: text.chars().count() / 4,
            chunk_count: chunks.len(),
            section_count: sections.len(),
            figure_count: figures.len(),
            table_count: tables.len(),
            reference_count: references.len(),
        },
        outline,
        sections,
        figures,
        tables,
        references,
        evidence_index,
        llm_report,
        cache: None,
    }
}

pub(crate) fn strip_for_output(
    mut paper: AcademicProgressivePaper,
    include_section_text: bool,
    max_chars: Option<usize>,
) -> AcademicProgressivePaper {
    if !include_section_text {
        for section in &mut paper.sections {
            section.clean_text = None;
        }
    }
    if let Some(max_chars) = max_chars {
        for section in &mut paper.sections {
            if let Some(text) = section.clean_text.as_mut() {
                if text.chars().count() > max_chars {
                    *text = text.chars().take(max_chars).collect();
                }
            }
        }
    }
    paper
}

fn credible_sections(
    results: &[ChunkResult],
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
) -> Vec<SectionCandidate> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for result in results {
        for candidate in &result.output.section_candidates {
            if !evidence.contains_key(&candidate.start_anchor) {
                continue;
            }
            if candidate.confidence.unwrap_or(0.0) < 0.5 {
                continue;
            }
            if !looks_like_section_heading(&candidate.title) {
                continue;
            }
            let key = normalize_heading(&candidate.title);
            if seen.insert(key) {
                out.push(candidate.clone());
            }
        }
    }
    out.sort_by_key(|candidate| {
        evidence
            .get(&candidate.start_anchor)
            .map(|span| span.char_start)
            .unwrap_or(usize::MAX)
    });
    out
}

fn sections_from_candidates(
    text: &str,
    chunks: &[Chunk],
    candidates: &[SectionCandidate],
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
) -> Vec<AcademicProgressiveSection> {
    let mut out = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(start) = evidence.get(&candidate.start_anchor).cloned() else {
            continue;
        };
        let end = candidates
            .get(index + 1)
            .and_then(|next| evidence.get(&next.start_anchor))
            .map(|span| span.char_start)
            .unwrap_or(text.len());
        let clean_text = safe_slice(text, start.char_start, end).trim().to_string();
        let local_chunks = chunks
            .iter()
            .filter(|chunk| chunk.end_char >= start.char_start && chunk.start_char <= end)
            .map(|chunk| chunk.id.clone())
            .collect::<Vec<_>>();
        out.push(AcademicProgressiveSection {
            section_id: section_id(&candidate.title, index),
            title: candidate
                .title
                .trim()
                .trim_start_matches('#')
                .trim()
                .to_string(),
            level: candidate
                .level
                .unwrap_or_else(|| infer_level(&candidate.title)),
            source_spans: vec![start],
            summary: String::new(),
            key_points: Vec::new(),
            local_chunks,
            figures: Vec::new(),
            tables: Vec::new(),
            references: Vec::new(),
            clean_text: Some(clean_text),
        });
    }
    out
}

fn fallback_section(
    text: &str,
    chunks: &[Chunk],
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
) -> AcademicProgressiveSection {
    AcademicProgressiveSection {
        section_id: "sec_000_paper".to_string(),
        title: "Paper".to_string(),
        level: 1,
        source_spans: evidence.values().next().into_iter().cloned().collect(),
        summary: String::new(),
        key_points: Vec::new(),
        local_chunks: chunks.iter().map(|chunk| chunk.id.clone()).collect(),
        figures: Vec::new(),
        tables: Vec::new(),
        references: Vec::new(),
        clean_text: Some(text.to_string()),
    }
}

fn attach_chunk_digests(sections: &mut [AcademicProgressiveSection], results: &[ChunkResult]) {
    let digest_by_chunk = results
        .iter()
        .filter_map(|result| {
            result
                .output
                .local_digest
                .as_ref()
                .map(|digest| (result.report.chunk_id.clone(), digest.text.clone()))
        })
        .collect::<HashMap<_, _>>();
    for section in sections {
        let digests = section
            .local_chunks
            .iter()
            .filter_map(|chunk| digest_by_chunk.get(chunk))
            .take(3)
            .cloned()
            .collect::<Vec<_>>();
        section.summary = digests.join(" ");
        section.key_points = digests;
    }
}

fn entities_from_results(
    results: &[ChunkResult],
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
) -> (
    Vec<AcademicProgressiveFigure>,
    Vec<AcademicProgressiveTable>,
    Vec<AcademicProgressiveReference>,
) {
    let mut figures = Vec::new();
    let mut tables = Vec::new();
    let mut references = Vec::new();
    for result in results {
        for entity in &result.output.entities {
            match entity.kind.as_str() {
                "figure" => figures.push(figure(entity, evidence, figures.len())),
                "table" => tables.push(table(entity, evidence, tables.len())),
                "reference" => references.push(reference(entity, evidence, references.len())),
                _ => {}
            }
        }
    }
    (figures, tables, references)
}

fn figure(
    entity: &Entity,
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
    index: usize,
) -> AcademicProgressiveFigure {
    AcademicProgressiveFigure {
        figure_id: format!("fig_{index:03}"),
        label: nonempty(&entity.label, "Figure"),
        caption: nonempty(&entity.text, &entity.label),
        source_spans: evidence
            .get(&entity.anchor_id)
            .into_iter()
            .cloned()
            .collect(),
        artifact_path: None,
    }
}

fn table(
    entity: &Entity,
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
    index: usize,
) -> AcademicProgressiveTable {
    AcademicProgressiveTable {
        table_id: format!("tbl_{index:03}"),
        label: nonempty(&entity.label, "Table"),
        caption: nonempty(&entity.text, &entity.label),
        source_spans: evidence
            .get(&entity.anchor_id)
            .into_iter()
            .cloned()
            .collect(),
        artifact_path: None,
    }
}

fn reference(
    entity: &Entity,
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
    index: usize,
) -> AcademicProgressiveReference {
    AcademicProgressiveReference {
        reference_id: format!("ref_{index:03}"),
        text: nonempty(&entity.text, &entity.label),
        source_spans: evidence
            .get(&entity.anchor_id)
            .into_iter()
            .cloned()
            .collect(),
    }
}

fn reference_tail_items(
    source: Option<&PdfProgressiveSourceBundle>,
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
    offset: usize,
) -> Vec<AcademicProgressiveReference> {
    let Some(source) = source else {
        return Vec::new();
    };
    source
        .reference_tail
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().count() >= 25)
        .take(120)
        .enumerate()
        .map(|(index, line)| AcademicProgressiveReference {
            reference_id: format!("ref_{:03}", offset + index),
            text: line.to_string(),
            source_spans: evidence.values().last().into_iter().cloned().collect(),
        })
        .collect()
}

fn infer_metadata(
    text: &str,
    evidence: &BTreeMap<String, AcademicProgressiveEvidenceSpan>,
) -> AcademicProgressiveMetadata {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let title = lines
        .iter()
        .find(|line| line.chars().count() >= 8 && !line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim().to_string());
    let abstract_text = lines.windows(2).find_map(|pair| {
        pair[0]
            .trim_start_matches('#')
            .trim()
            .eq_ignore_ascii_case("abstract")
            .then(|| pair[1].to_string())
    });
    AcademicProgressiveMetadata {
        title,
        authors: Vec::new(),
        abstract_text,
        keywords: Vec::new(),
        identifiers: BTreeMap::new(),
        source_spans: evidence.values().next().into_iter().cloned().collect(),
    }
}

fn accepted_patch_count(text: &str, chunks: &[Chunk], results: &[ChunkResult]) -> usize {
    patch_counts(text, chunks, results).0
}

fn rejected_patch_count(text: &str, chunks: &[Chunk], results: &[ChunkResult]) -> usize {
    patch_counts(text, chunks, results).1
}

fn patch_counts(text: &str, chunks: &[Chunk], results: &[ChunkResult]) -> (usize, usize) {
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    let mut edited = 0usize;
    let total_budget = text.len().saturating_mul(3) / 100;
    let chunk_by_id = chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<HashMap<_, _>>();
    for result in results {
        let Some(chunk) = chunk_by_id.get(result.report.chunk_id.as_str()) else {
            rejected += result.output.patches.len();
            continue;
        };
        let chunk_budget = chunk.text.len().saturating_mul(2) / 100;
        let mut chunk_edited = 0usize;
        for patch in &result.output.patches {
            let len = patch.original_excerpt.len();
            if patch.confidence.unwrap_or(0.0) < 0.55
                || patch.original_excerpt.trim().is_empty()
                || !chunk.text.contains(&patch.original_excerpt)
                || chunk_edited.saturating_add(len) > chunk_budget
                || edited.saturating_add(len) > total_budget
            {
                rejected += 1;
            } else {
                accepted += 1;
                chunk_edited += len;
                edited += len;
            }
        }
    }
    (accepted, rejected)
}

fn looks_like_section_heading(line: &str) -> bool {
    let trimmed = line.trim().trim_start_matches('#').trim();
    let lower = trimmed.to_ascii_lowercase();
    if trimmed.chars().count() > 120 || trimmed.contains("http://") || trimmed.contains('@') {
        return false;
    }
    if lower.starts_with("fig.")
        || lower.starts_with("figure ")
        || lower.starts_with("table ")
        || lower.starts_with("equation ")
        || lower.starts_with("doi")
    {
        return false;
    }
    matches!(
        lower.as_str(),
        "abstract"
            | "introduction"
            | "related work"
            | "background"
            | "method"
            | "methods"
            | "approach"
            | "experiments"
            | "results"
            | "discussion"
            | "conclusion"
            | "references"
            | "acknowledgements"
            | "acknowledgments"
            | "appendix"
    ) || trimmed.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn normalize_heading(value: &str) -> String {
    value
        .trim_start_matches('#')
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn section_id(title: &str, index: usize) -> String {
    let slug = title
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    format!(
        "sec_{index:03}_{}",
        slug.chars().take(36).collect::<String>()
    )
}

fn infer_level(title: &str) -> u8 {
    let hashes = title.chars().take_while(|ch| *ch == '#').count();
    if hashes > 0 {
        return hashes.min(6) as u8;
    }
    1
}

fn safe_slice(text: &str, start: usize, end: usize) -> &str {
    let start = floor_boundary(text, start);
    let end = floor_boundary(text, end.min(text.len())).max(start);
    &text[start..end]
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}
