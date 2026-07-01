use pdf_oxide::geometry::Rect;

use super::{
    manifest::SerializableTable, quality::looks_like_text_flow_table, IMAGE_MIN_BBOX_AREA,
    IMAGE_MIN_BBOX_HEIGHT, IMAGE_MIN_BBOX_WIDTH, IMAGE_MIN_PIXEL_AREA, TABLE_MAX_EMPTY_RATIO,
};

pub(crate) fn image_filter_reason(width: u32, height: u32, bbox: Option<Rect>) -> Option<String> {
    let pixel_area = u64::from(width) * u64::from(height);
    if pixel_area < IMAGE_MIN_PIXEL_AREA {
        return Some("tiny_pixel".to_string());
    }
    if let Some(rect) = bbox {
        let area = rect.width.abs() * rect.height.abs();
        let large_area = area >= IMAGE_MIN_BBOX_AREA;
        let large_shape =
            rect.width.abs() >= IMAGE_MIN_BBOX_WIDTH && rect.height.abs() >= IMAGE_MIN_BBOX_HEIGHT;
        if !large_area && !large_shape {
            return Some("small_bbox".to_string());
        }
    }
    None
}

pub(super) fn table_filter_reason(table: &SerializableTable) -> Option<String> {
    if table.col_count < 2 {
        return Some("too_few_columns".to_string());
    }
    if table.non_empty_cells < 4 {
        return Some("too_few_cells".to_string());
    }
    if table.rows.len() < 2 {
        return Some("single_row".to_string());
    }
    if table.empty_cell_ratio >= TABLE_MAX_EMPTY_RATIO {
        return Some("sparse".to_string());
    }
    if !table.real_grid {
        return Some("not_real_grid".to_string());
    }
    if looks_like_text_flow_table(table) {
        return Some("text_flow_fallback".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::manifest::{SerializableTableCell, SerializableTableRow};

    #[test]
    fn filters_two_column_text_flow_table() {
        let mut table = SerializableTable {
            rows: (0..10)
                .map(|index| SerializableTableRow {
                    is_header: index == 0,
                    cells: vec![
                        SerializableTableCell {
                            text: "This is a long prose-like row that should not be a table"
                                .to_string(),
                            colspan: 1,
                            rowspan: 1,
                            is_header: index == 0,
                            bbox: None,
                        },
                        SerializableTableCell {
                            text: "12".to_string(),
                            colspan: 1,
                            rowspan: 1,
                            is_header: index == 0,
                            bbox: None,
                        },
                    ],
                })
                .collect(),
            has_header: true,
            col_count: 2,
            row_count: 0,
            non_empty_cells: 0,
            total_cells: 0,
            empty_cell_ratio: 0.0,
            real_grid: false,
            bbox: None,
        };
        table.refresh_stats();

        assert_eq!(
            table_filter_reason(&table).as_deref(),
            Some("text_flow_fallback")
        );
    }
}
