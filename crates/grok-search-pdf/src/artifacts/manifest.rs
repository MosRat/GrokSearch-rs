use pdf_oxide::geometry::Rect;
use pdf_oxide::structure::table_extractor::{Table, TableCell, TableRow};
use serde::Serialize;

use super::quality::TableQuality;

#[derive(Debug)]
pub(super) struct ImageCandidate {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) image_index: usize,
    pub(super) filename: String,
    pub(super) png: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixel_area: u64,
    pub(super) bbox: Option<Rect>,
    pub(super) bbox_area: Option<f32>,
    pub(super) color_space: String,
    pub(super) bits_per_component: u8,
    pub(super) rotation_degrees: i32,
    pub(super) matrix: [f32; 6],
    pub(super) sha256: String,
    pub(super) filter_reason: Option<String>,
    pub(super) sort_area: f32,
}

#[derive(Debug)]
pub(super) struct TableCandidate {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) table_index: usize,
    pub(super) strategy: TableStrategy,
    pub(super) table: SerializableTable,
}

#[derive(Debug, Serialize)]
pub(super) struct ImageManifest<'a> {
    pub(super) kind: &'a str,
    pub(super) status: &'a str,
    pub(super) pipeline: Vec<&'a str>,
    pub(super) pages: usize,
    pub(super) total_candidates: usize,
    pub(super) written: usize,
    pub(super) filtered: usize,
    pub(super) errors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) page_summaries: Vec<ImagePageSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) component_groups: Vec<ImageComponentGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) caption_only_figures: Vec<CaptionOnlyFigure>,
    pub(super) entries: Vec<ImageManifestEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct ImagePageSummary {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) candidates: usize,
    pub(super) written: usize,
    pub(super) filtered: usize,
    pub(super) errors: usize,
    pub(super) figure_captions: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct ImageComponentGroup {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bbox: Option<Rect>,
    pub(super) filter_reasons: Vec<String>,
    pub(super) image_indexes: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub(super) struct CaptionOnlyFigure {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) caption: String,
    pub(super) reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ImageManifestEntry {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) image_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path: Option<String>,
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) filter_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) dedupe_of: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) diagnostics: Vec<String>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixel_area: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bbox: Option<Rect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bbox_area: Option<f32>,
    pub(super) color_space: String,
    pub(super) bits_per_component: u8,
    pub(super) rotation_degrees: i32,
    pub(super) matrix: [f32; 6],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct TableManifest<'a> {
    pub(super) kind: &'a str,
    pub(super) status: &'a str,
    pub(super) pipeline: Vec<&'a str>,
    pub(super) pages: usize,
    pub(super) total_candidates: usize,
    pub(super) written: usize,
    pub(super) filtered: usize,
    pub(super) errors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) page_summaries: Vec<TablePageSummary>,
    pub(super) entries: Vec<TableManifestEntry>,
}

#[derive(Debug, Serialize)]
pub(super) struct TablePageSummary {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) candidates: usize,
    pub(super) written: usize,
    pub(super) filtered: usize,
    pub(super) errors: usize,
    pub(super) profile_counts: TableProfileCounts,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct TableProfileCounts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) default: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) strict: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relaxed: Option<usize>,
}

impl TableProfileCounts {
    pub(super) fn empty() -> Self {
        Self {
            default: None,
            strict: None,
            relaxed: None,
        }
    }

    pub(super) fn error() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Serialize)]
pub(super) struct TableManifestEntry {
    pub(super) page_index: usize,
    pub(super) page_number: usize,
    pub(super) table_index: usize,
    pub(super) strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) path: Option<String>,
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) filter_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bytes: Option<u64>,
    pub(super) quality: TableQuality,
    pub(super) profile_counts: TableProfileCounts,
    pub(super) table: SerializableTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TableStrategy {
    PdfOxideGeometry,
    MarkdownTextFallback,
}

impl TableStrategy {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::PdfOxideGeometry => "pdf_oxide_geometry",
            Self::MarkdownTextFallback => "markdown_text_fallback",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SerializableTable {
    pub(super) rows: Vec<SerializableTableRow>,
    pub(super) has_header: bool,
    pub(super) col_count: usize,
    pub(super) row_count: usize,
    pub(super) non_empty_cells: usize,
    pub(super) total_cells: usize,
    pub(super) empty_cell_ratio: f32,
    pub(super) real_grid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bbox: Option<Rect>,
}

impl SerializableTable {
    pub(super) fn empty() -> Self {
        Self {
            rows: Vec::new(),
            has_header: false,
            col_count: 0,
            row_count: 0,
            non_empty_cells: 0,
            total_cells: 0,
            empty_cell_ratio: 0.0,
            real_grid: false,
            bbox: None,
        }
    }

    pub(super) fn from_pdf_table(table: &Table) -> Self {
        let mut serializable = Self {
            rows: table
                .rows
                .iter()
                .map(SerializableTableRow::from_pdf_row)
                .collect(),
            has_header: table.has_header,
            col_count: table.col_count,
            row_count: 0,
            non_empty_cells: 0,
            total_cells: 0,
            empty_cell_ratio: 0.0,
            real_grid: table.is_real_grid(),
            bbox: table.bbox,
        };
        serializable.refresh_stats();
        serializable
    }

    pub(super) fn refresh_stats(&mut self) {
        self.row_count = self.rows.len();
        self.col_count = self.col_count.max(
            self.rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        );
        self.total_cells = self.rows.iter().map(|row| row.cells.len()).sum();
        self.non_empty_cells = self
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .filter(|cell| !cell.text.trim().is_empty())
            .count();
        self.empty_cell_ratio = if self.total_cells == 0 {
            1.0
        } else {
            (self.total_cells - self.non_empty_cells) as f32 / self.total_cells as f32
        };
        self.real_grid = self.col_count >= 2
            && self.row_count >= 2
            && rows_with_two_or_more_filled_cells(&self.rows) * 2 >= self.row_count;
    }

    pub(super) fn to_markdown(&self) -> String {
        let cols = self.col_count.max(
            self.rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        );
        if cols == 0 {
            return String::new();
        }
        let mut lines = Vec::new();
        let header = self
            .rows
            .first()
            .map(|row| row.cells.as_slice())
            .unwrap_or(&[]);
        lines.push(markdown_row(header, cols));
        lines.push(format!(
            "|{}|",
            std::iter::repeat("---")
                .take(cols)
                .collect::<Vec<_>>()
                .join("|")
        ));
        for row in self.rows.iter().skip(1) {
            lines.push(markdown_row(&row.cells, cols));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

fn rows_with_two_or_more_filled_cells(rows: &[SerializableTableRow]) -> usize {
    rows.iter()
        .filter(|row| {
            row.cells
                .iter()
                .filter(|cell| !cell.text.trim().is_empty())
                .count()
                >= 2
        })
        .count()
}

fn markdown_row(cells: &[SerializableTableCell], cols: usize) -> String {
    let mut values = Vec::with_capacity(cols);
    for idx in 0..cols {
        let value = cells
            .get(idx)
            .map(|cell| markdown_escape(cell.text.trim()))
            .unwrap_or_default();
        values.push(value);
    }
    format!("|{}|", values.join("|"))
}

fn markdown_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SerializableTableRow {
    pub(super) is_header: bool,
    pub(super) cells: Vec<SerializableTableCell>,
}

impl SerializableTableRow {
    fn from_pdf_row(row: &TableRow) -> Self {
        Self {
            is_header: row.is_header,
            cells: row
                .cells
                .iter()
                .map(SerializableTableCell::from_pdf_cell)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SerializableTableCell {
    pub(super) text: String,
    pub(super) colspan: u32,
    pub(super) rowspan: u32,
    pub(super) is_header: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) bbox: Option<Rect>,
}

impl SerializableTableCell {
    fn from_pdf_cell(cell: &TableCell) -> Self {
        Self {
            text: cell.text.clone(),
            colspan: cell.colspan,
            rowspan: cell.rowspan,
            is_header: cell.is_header,
            bbox: cell.bbox,
        }
    }
}
