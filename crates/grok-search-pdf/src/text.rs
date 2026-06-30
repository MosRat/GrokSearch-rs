use std::collections::{HashMap, HashSet};

use grok_search_types::{GrokSearchError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextProcessingMode {
    None,
    Light,
    Clean,
}

impl TextProcessingMode {
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value
            .unwrap_or("clean")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "clean" => Ok(Self::Clean),
            "none" | "raw" => Ok(Self::None),
            "light" => Ok(Self::Light),
            other => Err(GrokSearchError::InvalidParams(format!(
                "text_processing_mode must be one of none, light, clean; got {other}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Light => "light",
            Self::Clean => "clean",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextSignals {
    pub line_count: usize,
    pub repeated_line_count: usize,
    pub short_line_ratio: f32,
    pub figure_caption_count: usize,
    pub table_caption_count: usize,
    pub url_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCleanResult {
    pub content: String,
    pub warnings: Vec<String>,
}

pub fn analyze_text_signals(content: &str) -> TextSignals {
    let mut counts = HashMap::<String, usize>::new();
    let mut line_count = 0usize;
    let mut short_lines = 0usize;
    let mut figure_caption_count = 0usize;
    let mut table_caption_count = 0usize;
    let mut url_count = 0usize;

    for line in content.lines() {
        let normalized = normalize_repeated_line_key(line);
        if !normalized.is_empty() {
            *counts.entry(normalized).or_default() += 1;
        }
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            line_count += 1;
            if trimmed.chars().count() <= 18 {
                short_lines += 1;
            }
            let lower = trimmed.to_ascii_lowercase();
            if lower.starts_with("fig.") || lower.starts_with("figure ") {
                figure_caption_count += 1;
            }
            if lower.starts_with("table ") {
                table_caption_count += 1;
            }
            if lower.contains("http://") || lower.contains("https://") {
                url_count += 1;
            }
        }
    }

    let repeated_line_count = counts.values().filter(|count| **count >= 3).count();
    let short_line_ratio = if line_count == 0 {
        0.0
    } else {
        short_lines as f32 / line_count as f32
    };

    TextSignals {
        line_count,
        repeated_line_count,
        short_line_ratio,
        figure_caption_count,
        table_caption_count,
        url_count,
    }
}

pub fn clean_text(content: &str, mode: TextProcessingMode) -> TextCleanResult {
    if mode == TextProcessingMode::None {
        return TextCleanResult {
            content: content.to_string(),
            warnings: Vec::new(),
        };
    }

    let repeated = repeated_noise_lines(content);
    let mut warnings = Vec::new();
    if !repeated.is_empty() {
        warnings.push(format!("removed {} repeated layout lines", repeated.len()));
    }

    let without_noise = content
        .lines()
        .filter(|line| {
            let key = normalize_repeated_line_key(line);
            key.is_empty() || !repeated.contains(&key)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let repaired = repair_hyphenation(&without_noise);
    let content = if mode == TextProcessingMode::Clean {
        normalize_paragraphs(&repaired)
    } else {
        normalize_blank_lines(&repaired)
    };

    TextCleanResult { content, warnings }
}

fn repeated_noise_lines(content: &str) -> HashSet<String> {
    let mut counts = HashMap::<String, usize>::new();
    for line in content.lines() {
        let key = normalize_repeated_line_key(line);
        if key.len() >= 3 && key.chars().count() <= 90 {
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter_map(|(line, count)| (count >= 3 && looks_like_layout_noise(&line)).then_some(line))
        .collect()
}

fn looks_like_layout_noise(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("fig.") || lower.starts_with("figure ") || lower.starts_with("table ") {
        return false;
    }
    if lower.contains("http://") || lower.contains("https://") {
        return false;
    }
    if line
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch.is_whitespace())
    {
        return true;
    }
    lower.contains("proceedings")
        || lower.contains("arxiv")
        || lower.contains("copyright")
        || lower.contains("preprint")
        || lower.contains("conference")
        || lower.contains("journal")
}

fn normalize_repeated_line_key(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn repair_hyphenation(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed_end = line.trim_end();
        if let Some(prefix) = trimmed_end.strip_suffix('-') {
            if let Some(next) = lines.peek() {
                let next_trimmed = next.trim_start();
                if starts_with_lowercase_letter(next_trimmed) && !ends_with_whitespace(prefix) {
                    out.push_str(prefix);
                    out.push_str(next_trimmed);
                    lines.next();
                    out.push('\n');
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    trim_trailing_newlines(out)
}

fn normalize_paragraphs(content: &str) -> String {
    let mut paragraphs = Vec::<String>::new();
    let mut current = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            push_paragraph(&mut paragraphs, &mut current);
            continue;
        }
        if should_keep_as_own_line(trimmed) {
            push_paragraph(&mut paragraphs, &mut current);
            paragraphs.push(trimmed.to_string());
            continue;
        }
        if current.is_empty() {
            current.push_str(trimmed);
        } else if should_join_line(&current, trimmed) {
            current.push(' ');
            current.push_str(trimmed);
        } else {
            push_paragraph(&mut paragraphs, &mut current);
            current.push_str(trimmed);
        }
    }
    push_paragraph(&mut paragraphs, &mut current);

    trim_trailing_newlines(paragraphs.join("\n\n"))
}

fn normalize_blank_lines(content: &str) -> String {
    let mut out = Vec::new();
    let mut last_blank = false;
    for line in content.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !last_blank {
                out.push(String::new());
            }
        } else {
            out.push(line.trim_end().to_string());
        }
        last_blank = is_blank;
    }
    trim_trailing_newlines(out.join("\n"))
}

fn should_join_line(current: &str, next: &str) -> bool {
    if current.ends_with('.') || current.ends_with(':') || current.ends_with(';') {
        return false;
    }
    if should_keep_as_own_line(next) {
        return false;
    }
    let next_chars = next.chars().count();
    next_chars <= 96 || starts_with_lowercase_letter(next)
}

fn should_keep_as_own_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with('#')
        || lower.starts_with('|')
        || lower.starts_with("fig.")
        || lower.starts_with("figure ")
        || lower.starts_with("table ")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || line.ends_with(':')
        || is_probable_equation_or_reference(line)
}

fn is_probable_equation_or_reference(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with('[') && trimmed.contains(']') {
        return true;
    }
    let math_marks = trimmed
        .chars()
        .filter(|ch| matches!(ch, '=' | '<' | '>' | '+' | '-' | '/'))
        .count();
    math_marks >= 2
}

fn ends_with_whitespace(value: &str) -> bool {
    value
        .chars()
        .next_back()
        .map(|ch| ch.is_whitespace())
        .unwrap_or(false)
}

fn starts_with_lowercase_letter(value: &str) -> bool {
    value
        .chars()
        .find(|ch| ch.is_alphabetic())
        .map(|ch| ch.is_lowercase())
        .unwrap_or(false)
}

fn push_paragraph(paragraphs: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        paragraphs.push(trimmed.to_string());
    }
    current.clear();
}

fn trim_trailing_newlines(mut value: String) -> String {
    while value.ends_with('\n') {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_processing_modes() {
        assert_eq!(
            TextProcessingMode::parse(None).unwrap(),
            TextProcessingMode::Clean
        );
        assert_eq!(
            TextProcessingMode::parse(Some("none")).unwrap(),
            TextProcessingMode::None
        );
        assert!(TextProcessingMode::parse(Some("heavy")).is_err());
    }

    #[test]
    fn clean_repairs_hyphenation_and_removes_repeated_noise() {
        let raw = [
            "arXiv preprint",
            "Trans-",
            "former models work",
            "",
            "arXiv preprint",
            "This line",
            "continues softly",
            "arXiv preprint",
        ]
        .join("\n");

        let cleaned = clean_text(&raw, TextProcessingMode::Clean);
        assert!(cleaned.content.contains("Transformer models work"));
        assert!(cleaned.content.contains("This line continues softly"));
        assert!(!cleaned.content.contains("arXiv preprint"));
        assert_eq!(cleaned.warnings, vec!["removed 1 repeated layout lines"]);
    }

    #[test]
    fn clean_keeps_captions_urls_and_equations_separate() {
        let raw = [
            "Fig. 1: Overview",
            "https://example.com/code",
            "x = y = z",
            "short",
            "continuation",
        ]
        .join("\n");

        let cleaned = clean_text(&raw, TextProcessingMode::Clean);
        assert!(cleaned
            .content
            .contains("Fig. 1: Overview\n\nhttps://example.com/code"));
        assert!(cleaned.content.contains("x = y = z"));
        assert!(cleaned.content.contains("short continuation"));
    }
}
