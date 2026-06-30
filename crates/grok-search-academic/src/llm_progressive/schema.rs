use grok_search_types::{AcademicProgressiveChunkReport, GrokSearchError, Result};
use serde::{Deserialize, Serialize};

use super::chunking::Chunk;

#[derive(Debug, Clone)]
pub(crate) struct ChunkResult {
    pub output: ChunkOutput,
    pub report: AcademicProgressiveChunkReport,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ChunkOutput {
    #[serde(default)]
    pub patches: Vec<Patch>,
    #[serde(default)]
    pub section_candidates: Vec<SectionCandidate>,
    #[serde(default)]
    pub local_digest: Option<LocalDigest>,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Patch {
    pub kind: String,
    #[serde(default)]
    pub anchor_id: String,
    #[serde(default)]
    pub original_excerpt: String,
    #[serde(default)]
    pub replacement: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SectionCandidate {
    pub title: String,
    #[serde(default)]
    pub level: Option<u8>,
    #[serde(default)]
    pub start_anchor: String,
    #[serde(default)]
    pub end_anchor: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct LocalDigest {
    pub text: String,
    #[serde(default)]
    pub anchors: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Entity {
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub anchor_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub confidence: Option<f32>,
}

pub(crate) fn parse_chunk_output(
    chunk: &Chunk,
    output: &str,
    latency_ms: u64,
    repaired: bool,
) -> Result<ChunkResult> {
    let json_text = extract_json_object(output);
    let parsed = serde_json::from_str::<ChunkOutput>(&json_text)
        .map_err(|err| GrokSearchError::Parse(format!("invalid progressive chunk JSON: {err}")))?;
    Ok(ChunkResult {
        report: AcademicProgressiveChunkReport {
            chunk_id: chunk.id.clone(),
            start_char: chunk.start_char,
            end_char: chunk.end_char,
            input_chars: chunk.text.chars().count(),
            output_chars: output.chars().count(),
            latency_ms,
            json_valid: true,
            repaired,
            fallback: false,
            warnings: {
                let mut warnings = parsed.warnings.clone();
                warnings.push(format!(
                    "line_range={}-{}",
                    chunk.start_line, chunk.end_line
                ));
                warnings
            },
        },
        output: parsed,
    })
}

pub(crate) fn fallback_chunk_result(error: String) -> ChunkResult {
    ChunkResult {
        output: ChunkOutput {
            patches: Vec::new(),
            section_candidates: Vec::new(),
            local_digest: Some(LocalDigest {
                text: "LLM chunk failed; use source text anchors for inspection.".to_string(),
                anchors: Vec::new(),
            }),
            entities: Vec::new(),
            warnings: vec![error.clone()],
        },
        report: AcademicProgressiveChunkReport {
            chunk_id: "unknown".to_string(),
            fallback: true,
            json_valid: false,
            warnings: vec![error],
            ..Default::default()
        },
    }
}

fn extract_json_object(text: &str) -> String {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim();
    let start = trimmed.find('{').unwrap_or(0);
    let end = trimmed
        .rfind('}')
        .map(|idx| idx + 1)
        .unwrap_or(trimmed.len());
    trimmed[start..end].to_string()
}
