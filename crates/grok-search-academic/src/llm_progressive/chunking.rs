use grok_search_types::AcademicProgressiveEvidenceSpan;

#[derive(Debug, Clone)]
pub(crate) struct Chunk {
    pub id: String,
    pub text: String,
    pub start_char: usize,
    pub end_char: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub anchors: Vec<AcademicProgressiveEvidenceSpan>,
}

pub(crate) fn build_chunks(text: &str, max_chunk_chars: usize, overlap_chars: usize) -> Vec<Chunk> {
    let ranges = paragraph_ranges(text);
    let line_starts = line_starts(text);
    let mut chunks = Vec::new();
    let mut start = ranges.first().map(|range| range.0).unwrap_or(0);
    let mut end = start;
    for (para_start, para_end) in ranges {
        if end > start && para_end.saturating_sub(start) > max_chunk_chars {
            chunks.push(build_chunk(text, chunks.len(), start, end, &line_starts));
            start = floor_char_boundary(text, end.saturating_sub(overlap_chars));
        } else if end == start {
            start = para_start;
        }
        end = para_end;
    }
    if end > start {
        chunks.push(build_chunk(text, chunks.len(), start, end, &line_starts));
    }
    chunks
}

fn paragraph_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx + 1 < bytes.len() {
        if bytes[idx] == b'\n' && bytes[idx + 1] == b'\n' {
            if idx > cursor {
                ranges.push((cursor, idx));
            }
            idx += 2;
            cursor = idx;
        } else {
            idx += 1;
        }
    }
    if cursor < text.len() {
        ranges.push((cursor, text.len()));
    }
    if ranges.is_empty() {
        ranges.push((0, text.len()));
    }
    ranges
}

fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' && idx + 1 < text.len() {
            out.push(idx + 1);
        }
    }
    out
}

fn build_chunk(
    text: &str,
    index: usize,
    start_char: usize,
    end_char: usize,
    line_starts: &[usize],
) -> Chunk {
    let start_line = byte_to_line(line_starts, start_char);
    let end_line = byte_to_line(line_starts, end_char.saturating_sub(1));
    let anchors = anchors_for_lines(text, line_starts, start_line, end_line, index);
    Chunk {
        id: format!("chunk_{index:04}"),
        text: text[start_char..end_char].to_string(),
        start_char,
        end_char,
        start_line: start_line + 1,
        end_line: end_line + 1,
        anchors,
    }
}

fn byte_to_line(line_starts: &[usize], char_index: usize) -> usize {
    match line_starts.binary_search(&char_index) {
        Ok(index) => index,
        Err(index) => index.saturating_sub(1),
    }
}

fn anchors_for_lines(
    text: &str,
    line_starts: &[usize],
    start_line: usize,
    end_line: usize,
    chunk_index: usize,
) -> Vec<AcademicProgressiveEvidenceSpan> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut anchors = Vec::new();
    for line_index in start_line..=end_line.min(lines.len().saturating_sub(1)) {
        let line = lines[line_index].trim();
        if line.is_empty() {
            continue;
        }
        let char_start = *line_starts.get(line_index).unwrap_or(&0);
        let char_end = char_start + lines[line_index].len();
        let excerpt = line.chars().take(240).collect::<String>();
        anchors.push(AcademicProgressiveEvidenceSpan {
            anchor_id: format!("c{chunk_index:04}_{:010x}", stable_hash(&excerpt)),
            page: None,
            line_start: line_index + 1,
            line_end: line_index + 1,
            char_start,
            char_end,
            excerpt,
        });
    }
    anchors
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_handles_non_ascii_overlap_boundaries() {
        let text = "摘要\n\n".repeat(100);
        let chunks = build_chunks(&text, 120, 17);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.text.is_empty()));
    }
}
