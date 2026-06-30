use grok_search_types::{AcademicProgressiveChunkReport, GrokSearchError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    attempts: u32,
    backoff_ms: u64,
) -> Result<ChunkResult> {
    let json_text = extract_json_object(output);
    let value = serde_json::from_str::<Value>(&json_text)
        .map_err(|err| GrokSearchError::Parse(format!("invalid progressive chunk JSON: {err}")))?;
    let parsed = normalize_chunk_value(value)?;
    Ok(ChunkResult {
        report: AcademicProgressiveChunkReport {
            chunk_id: chunk.id.clone(),
            start_char: chunk.start_char,
            end_char: chunk.end_char,
            input_chars: chunk.text.chars().count(),
            output_chars: output.chars().count(),
            latency_ms,
            attempts,
            backoff_ms,
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

pub(crate) fn fallback_chunk_result(
    chunk: Option<&Chunk>,
    error: String,
    attempts: u32,
    backoff_ms: u64,
) -> ChunkResult {
    let (chunk_id, start_char, end_char, input_chars, warnings) = match chunk {
        Some(chunk) => (
            chunk.id.clone(),
            chunk.start_char,
            chunk.end_char,
            chunk.text.chars().count(),
            vec![
                error.clone(),
                format!("line_range={}-{}", chunk.start_line, chunk.end_line),
            ],
        ),
        None => ("unknown".to_string(), 0, 0, 0, vec![error.clone()]),
    };
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
            chunk_id,
            start_char,
            end_char,
            input_chars,
            attempts,
            backoff_ms,
            fallback: true,
            json_valid: false,
            warnings,
            ..Default::default()
        },
    }
}

fn normalize_chunk_value(value: Value) -> Result<ChunkOutput> {
    if has_compact_keys(&value) {
        return expand_compact_or_micro(value);
    }
    let mut output = serde_json::from_value::<ChunkOutput>(value)
        .map_err(|err| GrokSearchError::Parse(format!("invalid progressive chunk shape: {err}")))?;
    if output.local_digest.is_none() {
        output.local_digest = Some(LocalDigest::default());
    }
    Ok(output)
}

fn has_compact_keys(value: &Value) -> bool {
    value
        .as_object()
        .map(|object| {
            object.contains_key("s") || object.contains_key("d") || object.contains_key("e")
        })
        .unwrap_or(false)
}

fn expand_compact_or_micro(value: Value) -> Result<ChunkOutput> {
    let object = value.as_object().ok_or_else(|| {
        GrokSearchError::Parse("progressive compact chunk must be a JSON object".to_string())
    })?;
    let micro = first_array_item(object.get("s"))
        .or_else(|| first_array_item(object.get("e")))
        .or_else(|| first_array_item(object.get("p")))
        .map(|item| item.is_array())
        .unwrap_or(false)
        || object
            .get("d")
            .map(|value| value.is_array())
            .unwrap_or(false);

    let sections = object
        .get("s")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .filter_map(|item| {
                    if micro {
                        section_from_micro(item)
                    } else {
                        section_from_compact(item)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let entities = object
        .get("e")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(16)
                .filter_map(|item| {
                    if micro {
                        entity_from_micro(item)
                    } else {
                        entity_from_compact(item)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let patches = object
        .get("p")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(3)
                .filter_map(|item| {
                    if micro {
                        patch_from_micro(item)
                    } else {
                        patch_from_compact(item)
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let local_digest = Some(if micro {
        digest_from_micro(object.get("d"))
    } else {
        LocalDigest {
            text: string_value(object.get("d")).unwrap_or_default(),
            anchors: string_array(object.get("da")).into_iter().take(6).collect(),
        }
    });
    let warnings = string_array(object.get("w")).into_iter().take(8).collect();
    Ok(ChunkOutput {
        patches,
        section_candidates: sections,
        local_digest,
        entities,
        warnings,
    })
}

fn first_array_item(value: Option<&Value>) -> Option<&Value> {
    value
        .and_then(Value::as_array)
        .and_then(|items| items.first())
}

fn section_from_compact(value: &Value) -> Option<SectionCandidate> {
    let object = value.as_object()?;
    Some(SectionCandidate {
        title: string_value(object.get("t"))?,
        level: Some(clamped_u8(object.get("l"), 2, 1, 6)),
        start_anchor: string_value(object.get("a"))?,
        end_anchor: None,
        confidence: Some(clamped_f32(object.get("c"), 0.5, 0.0, 1.0)),
    })
}

fn section_from_micro(value: &Value) -> Option<SectionCandidate> {
    let items = value.as_array()?;
    Some(SectionCandidate {
        title: string_at(items, 0)?,
        level: Some(clamped_u8(items.get(1), 2, 1, 6)),
        start_anchor: string_at(items, 2)?,
        end_anchor: None,
        confidence: Some(clamped_f32(items.get(3), 0.5, 0.0, 1.0)),
    })
}

fn entity_from_compact(value: &Value) -> Option<Entity> {
    let object = value.as_object()?;
    let kind = string_value(object.get("k"))?;
    if !matches!(kind.as_str(), "figure" | "table" | "reference" | "resource") {
        return None;
    }
    let label = string_value(object.get("x")).unwrap_or_default();
    Some(Entity {
        kind,
        label: label.clone(),
        anchor_id: string_value(object.get("a")).unwrap_or_default(),
        text: label,
        confidence: Some(clamped_f32(object.get("c"), 0.5, 0.0, 1.0)),
    })
}

fn entity_from_micro(value: &Value) -> Option<Entity> {
    let items = value.as_array()?;
    let kind = string_at(items, 0)?;
    if !matches!(kind.as_str(), "figure" | "table" | "reference" | "resource") {
        return None;
    }
    let label = string_at(items, 1).unwrap_or_default();
    Some(Entity {
        kind,
        label: label.clone(),
        anchor_id: string_at(items, 2).unwrap_or_default(),
        text: label,
        confidence: Some(clamped_f32(items.get(3), 0.5, 0.0, 1.0)),
    })
}

fn patch_from_compact(value: &Value) -> Option<Patch> {
    let object = value.as_object()?;
    let kind = string_value(object.get("k"))?;
    if !valid_patch_kind(&kind) {
        return None;
    }
    Some(Patch {
        kind,
        anchor_id: string_value(object.get("a")).unwrap_or_default(),
        original_excerpt: string_value(object.get("o")).unwrap_or_default(),
        replacement: nullable_string(object.get("r")),
        confidence: Some(clamped_f32(object.get("c"), 0.5, 0.0, 1.0)),
        reason: Some("compact_patch".to_string()),
    })
}

fn patch_from_micro(value: &Value) -> Option<Patch> {
    let items = value.as_array()?;
    let kind = string_at(items, 0)?;
    if !valid_patch_kind(&kind) {
        return None;
    }
    Some(Patch {
        kind,
        anchor_id: string_at(items, 1).unwrap_or_default(),
        original_excerpt: string_at(items, 2).unwrap_or_default(),
        replacement: nullable_string(items.get(3)),
        confidence: Some(clamped_f32(items.get(4), 0.5, 0.0, 1.0)),
        reason: Some("micro_patch".to_string()),
    })
}

fn digest_from_micro(value: Option<&Value>) -> LocalDigest {
    if let Some(items) = value.and_then(Value::as_array) {
        return LocalDigest {
            text: string_at(items, 0).unwrap_or_default(),
            anchors: (1..items.len().min(5))
                .filter_map(|index| string_at(items, index))
                .collect(),
        };
    }
    LocalDigest {
        text: string_value(value).unwrap_or_default(),
        anchors: Vec::new(),
    }
}

fn valid_patch_kind(kind: &str) -> bool {
    matches!(
        kind,
        "join_lines" | "dehyphenate" | "delete_noise_line" | "replace_small_span" | "mark_boundary"
    )
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| string_value(Some(item)))
                .collect()
        })
        .unwrap_or_default()
}

fn string_at(items: &[Value], index: usize) -> Option<String> {
    string_value(items.get(index))
}

fn string_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn nullable_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Null) | None => None,
        Some(value) => string_value(Some(value)),
    }
}

fn clamped_u8(value: Option<&Value>, default: u8, lower: u8, upper: u8) -> u8 {
    let parsed = value
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(default);
    parsed.clamp(lower, upper)
}

fn clamped_f32(value: Option<&Value>, default: f32, lower: f32, upper: f32) -> f32 {
    let parsed = value
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|value| value.parse::<f64>().ok()))
        })
        .map(|value| value as f32)
        .unwrap_or(default);
    parsed.clamp(lower, upper)
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

#[cfg(test)]
mod tests {
    use super::*;
    use grok_search_types::AcademicProgressiveEvidenceSpan;

    fn test_chunk() -> Chunk {
        Chunk {
            id: "paragraph_window_0000".to_string(),
            text: "Introduction\nThis paper studies attention.".to_string(),
            start_char: 10,
            end_char: 49,
            start_line: 2,
            end_line: 3,
            anchors: vec![AcademicProgressiveEvidenceSpan {
                anchor_id: "c0000_intro".to_string(),
                page: None,
                line_start: 2,
                line_end: 2,
                char_start: 10,
                char_end: 22,
                excerpt: "Introduction".to_string(),
            }],
        }
    }

    #[test]
    fn parses_compact_short_key_chunk_output() {
        let raw = r#"{"s":[{"t":"Introduction","l":1,"a":"c0000_intro","c":0.9}],"d":"local digest","da":["c0000_intro"],"e":[{"k":"resource","x":"https://example.com","a":"c0000_intro","c":0.7}],"p":[{"k":"join_lines","a":"c0000_intro","o":"Intro- duction","r":"Introduction","c":0.8}],"w":["ok"]}"#;

        let result =
            parse_chunk_output(&test_chunk(), raw, 12, false, 1, 0).expect("parse compact");

        assert!(result.report.json_valid);
        assert_eq!(result.output.section_candidates[0].title, "Introduction");
        assert_eq!(result.output.entities[0].kind, "resource");
        assert_eq!(
            result.output.patches[0].reason.as_deref(),
            Some("compact_patch")
        );
        assert_eq!(
            result.output.local_digest.as_ref().unwrap().anchors,
            vec!["c0000_intro"]
        );
    }

    #[test]
    fn parses_micro_tuple_chunk_output_and_preserves_fallback_span() {
        let raw = r#"{"s":[["Methods",2,"c0000_intro",0.8]],"d":["digest","c0000_intro"],"e":[["figure","Figure 1","c0000_intro",0.7]],"p":[],"w":[]}"#;

        let result = parse_chunk_output(&test_chunk(), raw, 5, false, 1, 0).expect("parse micro");
        assert_eq!(result.output.section_candidates[0].title, "Methods");
        assert_eq!(result.output.entities[0].label, "Figure 1");

        let fallback = fallback_chunk_result(Some(&test_chunk()), "bad json".to_string(), 3, 900);
        assert_eq!(fallback.report.chunk_id, "paragraph_window_0000");
        assert_eq!(fallback.report.start_char, 10);
        assert_eq!(fallback.report.end_char, 49);
        assert_eq!(fallback.report.attempts, 3);
        assert_eq!(fallback.report.backoff_ms, 900);
        assert_eq!(
            fallback.report.input_chars,
            "Introduction\nThis paper studies attention."
                .chars()
                .count()
        );
    }
}
