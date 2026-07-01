use serde::Serialize;

use super::manifest::SerializableTable;

#[derive(Debug, Clone, Serialize)]
pub(super) struct TableQuality {
    score: f32,
    row_count: usize,
    col_count: usize,
    non_empty_cells: usize,
    total_cells: usize,
    filled_cell_ratio: f32,
    real_grid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_reason: Option<String>,
}

impl TableQuality {
    pub(super) fn empty() -> Self {
        Self {
            score: 0.0,
            row_count: 0,
            col_count: 0,
            non_empty_cells: 0,
            total_cells: 0,
            filled_cell_ratio: 0.0,
            real_grid: false,
            filter_reason: None,
        }
    }

    pub(super) fn from_table(table: &SerializableTable, filter_reason: Option<&str>) -> Self {
        let filled_cell_ratio = if table.total_cells == 0 {
            0.0
        } else {
            table.non_empty_cells as f32 / table.total_cells as f32
        };
        let row_score = (table.row_count.min(6) as f32) / 6.0;
        let col_score = (table.col_count.min(6) as f32) / 6.0;
        let grid_score = if table.real_grid { 1.0 } else { 0.0 };
        let score = (filled_cell_ratio * 0.45)
            + (row_score * 0.2)
            + (col_score * 0.2)
            + (grid_score * 0.15);
        Self {
            score,
            row_count: table.row_count,
            col_count: table.col_count,
            non_empty_cells: table.non_empty_cells,
            total_cells: table.total_cells,
            filled_cell_ratio,
            real_grid: table.real_grid,
            filter_reason: filter_reason.map(ToString::to_string),
        }
    }
}

pub(super) fn looks_like_text_flow_table(table: &SerializableTable) -> bool {
    if table.col_count != 2 || table.row_count < 8 {
        return false;
    }
    let rows_with_long_first_cell = table
        .rows
        .iter()
        .filter(|row| {
            let first_len = row
                .cells
                .first()
                .map(|cell| cell.text.split_whitespace().count())
                .unwrap_or(0);
            let second_len = row
                .cells
                .get(1)
                .map(|cell| cell.text.split_whitespace().count())
                .unwrap_or(0);
            first_len >= 8 && second_len <= 4
        })
        .count();
    rows_with_long_first_cell * 2 >= table.row_count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_quality_records_filter_reason() {
        let table = SerializableTable::empty();
        let quality = TableQuality::from_table(&table, Some("too_few_columns"));

        assert_eq!(quality.score, 0.0);
        assert_eq!(quality.filter_reason.as_deref(), Some("too_few_columns"));
    }
}
