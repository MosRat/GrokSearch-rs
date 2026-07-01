use super::manifest::{
    SerializableTable, SerializableTableCell, SerializableTableRow, TableCandidate, TableStrategy,
};

pub(super) fn markdown_table_candidates(
    page_text: &str,
    page_index: usize,
    page_number: usize,
    first_table_index: usize,
) -> Vec<TableCandidate> {
    let mut candidates = Vec::new();
    let mut block = Vec::<String>::new();
    for line in page_text.lines().chain(std::iter::once("")) {
        if looks_like_markdown_table_row(line) {
            block.push(line.trim().to_string());
            continue;
        }
        push_markdown_table_candidate(
            &mut candidates,
            &mut block,
            page_index,
            page_number,
            first_table_index,
        );
    }
    candidates
}

fn push_markdown_table_candidate(
    candidates: &mut Vec<TableCandidate>,
    block: &mut Vec<String>,
    page_index: usize,
    page_number: usize,
    first_table_index: usize,
) {
    if block.len() < 3 {
        block.clear();
        return;
    }
    let separator_index = block
        .iter()
        .position(|line| looks_like_markdown_separator_row(line));
    let Some(separator_index) = separator_index else {
        block.clear();
        return;
    };
    if separator_index == 0 {
        block.clear();
        return;
    }

    let rows = block
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != separator_index)
        .map(|(index, line)| SerializableTableRow {
            is_header: index < separator_index,
            cells: split_markdown_table_row(line)
                .into_iter()
                .map(|text| SerializableTableCell {
                    text,
                    colspan: 1,
                    rowspan: 1,
                    is_header: index < separator_index,
                    bbox: None,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let mut table = SerializableTable {
        rows,
        has_header: true,
        col_count: 0,
        row_count: 0,
        non_empty_cells: 0,
        total_cells: 0,
        empty_cell_ratio: 0.0,
        real_grid: false,
        bbox: None,
    };
    table.refresh_stats();
    if table.col_count >= 2 {
        candidates.push(TableCandidate {
            page_index,
            page_number,
            table_index: first_table_index + candidates.len(),
            strategy: TableStrategy::MarkdownTextFallback,
            table,
        });
    }
    block.clear();
}

fn looks_like_markdown_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 3
}

fn looks_like_markdown_separator_row(line: &str) -> bool {
    if !looks_like_markdown_table_row(line) {
        return false;
    }
    split_markdown_table_row(line)
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

fn split_markdown_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().replace("\\|", "|"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::filters::table_filter_reason;

    #[test]
    fn extracts_markdown_table_fallback_candidate() {
        let text = r#"
Before
| Method | Score |
| --- | ---: |
| Base | 0.71 |
| Ours | 0.92 |
After
"#;

        let candidates = markdown_table_candidates(text, 0, 1, 3);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].table_index, 3);
        assert_eq!(candidates[0].strategy, TableStrategy::MarkdownTextFallback);
        assert_eq!(candidates[0].table.row_count, 3);
        assert_eq!(candidates[0].table.col_count, 2);
        assert!(table_filter_reason(&candidates[0].table).is_none());
    }
}
