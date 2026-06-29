use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use grok_search_content::{ensure_output_dir, reject_existing_path};
use grok_search_types::{AcademicParseArtifact, GrokSearchError, Result};
use pdf_oxide::geometry::Rect;
use pdf_oxide::structure::table_extractor::{Table, TableCell, TableRow};
use pdf_oxide::PdfDocument;
use serde::Serialize;
use sha2::{Digest, Sha256};

const IMAGE_MIN_PIXEL_AREA: u64 = 50_000;
const IMAGE_MIN_BBOX_AREA: f32 = 3_000.0;
const IMAGE_MIN_BBOX_WIDTH: f32 = 60.0;
const IMAGE_MIN_BBOX_HEIGHT: f32 = 40.0;
const IMAGE_MAX_PER_PAGE: usize = 10;
const TABLE_MAX_EMPTY_RATIO: f32 = 0.5;

pub fn extract_image_artifacts(
    doc: &PdfDocument,
    pages: usize,
    dir: Option<&str>,
) -> Result<AcademicParseArtifact> {
    let dir = require_output_dir("images", dir)?;
    let manifest_path = dir.join("images.json");
    reject_existing_path(&manifest_path, "images manifest")?;

    let mut entries = Vec::new();
    let mut page_kept = HashMap::<usize, usize>::new();
    let mut seen_hashes = HashSet::<String>::new();
    let mut output_files = Vec::<(PathBuf, Vec<u8>)>::new();
    let mut errors = 0usize;

    for page_index in 0..pages {
        match doc.extract_images(page_index) {
            Ok(images) => {
                let mut candidates = Vec::new();
                for (image_index, image) in images.iter().enumerate() {
                    let page_number = page_index + 1;
                    let width = image.width();
                    let height = image.height();
                    let pixel_area = u64::from(width) * u64::from(height);
                    let bbox = image.bbox().copied();
                    let bbox_area = bbox_area(bbox);
                    let matrix = image.matrix();
                    let color_space = format!("{:?}", image.color_space());
                    let bits_per_component = image.bits_per_component();
                    let rotation_degrees = image.rotation_degrees();

                    let png = match image.to_png_bytes() {
                        Ok(bytes) => bytes,
                        Err(err) => {
                            errors += 1;
                            entries.push(ImageManifestEntry {
                                page_index,
                                page_number,
                                image_index,
                                path: None,
                                status: "error".to_string(),
                                filter_reason: Some(format!("encode_png: {err}")),
                                width,
                                height,
                                pixel_area,
                                bbox,
                                bbox_area,
                                color_space,
                                bits_per_component,
                                rotation_degrees,
                                matrix,
                                bytes: None,
                                sha256: None,
                            });
                            continue;
                        }
                    };

                    let sha256 = sha256_hex(&png);
                    let filter_reason = image_filter_reason(width, height, bbox);
                    let sort_area = bbox_area.unwrap_or(pixel_area as f32);
                    candidates.push(ImageCandidate {
                        page_index,
                        page_number,
                        image_index,
                        filename: format!("page-{page_number:04}-image-{:03}.png", image_index + 1),
                        png,
                        width,
                        height,
                        pixel_area,
                        bbox,
                        bbox_area,
                        color_space,
                        bits_per_component,
                        rotation_degrees,
                        matrix,
                        sha256,
                        filter_reason,
                        sort_area,
                    });
                }

                candidates.sort_by(|left, right| {
                    right
                        .sort_area
                        .partial_cmp(&left.sort_area)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| right.pixel_area.cmp(&left.pixel_area))
                        .then_with(|| left.image_index.cmp(&right.image_index))
                });

                for candidate in candidates {
                    let kept_on_page = page_kept.entry(candidate.page_index).or_default();
                    let mut filter_reason = candidate.filter_reason.clone();
                    if filter_reason.is_none() && seen_hashes.contains(&candidate.sha256) {
                        filter_reason = Some("duplicate".to_string());
                    }
                    if filter_reason.is_none() && *kept_on_page >= IMAGE_MAX_PER_PAGE {
                        filter_reason = Some("page_limit".to_string());
                    }

                    let (status, path, bytes) = if filter_reason.is_some() {
                        (
                            "filtered".to_string(),
                            None,
                            Some((candidate.png.len()) as u64),
                        )
                    } else {
                        let target = dir.join(&candidate.filename);
                        reject_existing_path(&target, "image artifact")?;
                        *kept_on_page += 1;
                        seen_hashes.insert(candidate.sha256.clone());
                        output_files.push((target.clone(), candidate.png.clone()));
                        (
                            "written".to_string(),
                            Some(target.display().to_string()),
                            Some((candidate.png.len()) as u64),
                        )
                    };

                    entries.push(ImageManifestEntry {
                        page_index: candidate.page_index,
                        page_number: candidate.page_number,
                        image_index: candidate.image_index,
                        path,
                        status,
                        filter_reason,
                        width: candidate.width,
                        height: candidate.height,
                        pixel_area: candidate.pixel_area,
                        bbox: candidate.bbox,
                        bbox_area: candidate.bbox_area,
                        color_space: candidate.color_space,
                        bits_per_component: candidate.bits_per_component,
                        rotation_degrees: candidate.rotation_degrees,
                        matrix: candidate.matrix,
                        bytes,
                        sha256: Some(candidate.sha256),
                    });
                }
            }
            Err(err) => {
                errors += 1;
                entries.push(ImageManifestEntry {
                    page_index,
                    page_number: page_index + 1,
                    image_index: 0,
                    path: None,
                    status: "error".to_string(),
                    filter_reason: Some(format!("extract_page: {err}")),
                    width: 0,
                    height: 0,
                    pixel_area: 0,
                    bbox: None,
                    bbox_area: None,
                    color_space: String::new(),
                    bits_per_component: 0,
                    rotation_degrees: 0,
                    matrix: [0.0; 6],
                    bytes: None,
                    sha256: None,
                });
            }
        }
    }

    let written = entries.iter().filter(|entry| entry.status == "written").count();
    let filtered = entries
        .iter()
        .filter(|entry| entry.status == "filtered")
        .count();
    let status = artifact_status(entries.len(), written, filtered, errors);
    let manifest = ImageManifest {
        kind: "images",
        status: &status,
        pages,
        total_candidates: entries.len(),
        written,
        filtered,
        errors,
        entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| GrokSearchError::Parse(format!("serialize images manifest: {err}")))?;
    let mut total_bytes = manifest_bytes.len() as u64;

    for (path, bytes) in output_files {
        total_bytes += bytes.len() as u64;
        std::fs::write(&path, bytes)
            .map_err(|err| GrokSearchError::Io(format!("write image artifact {}: {err}", path.display())))?;
    }
    std::fs::write(&manifest_path, manifest_bytes).map_err(|err| {
        GrokSearchError::Io(format!(
            "write images manifest {}: {err}",
            manifest_path.display()
        ))
    })?;

    Ok(AcademicParseArtifact {
        kind: "images".to_string(),
        status,
        path: Some(manifest_path.display().to_string()),
        count: Some(written),
        bytes: Some(total_bytes),
        reason: artifact_reason(filtered, errors),
    })
}

pub fn extract_table_artifacts(
    doc: &PdfDocument,
    pages: usize,
    dir: Option<&str>,
) -> Result<AcademicParseArtifact> {
    let dir = require_output_dir("tables", dir)?;
    let manifest_path = dir.join("tables.json");
    reject_existing_path(&manifest_path, "tables manifest")?;

    let mut entries = Vec::new();
    let mut output_files = Vec::<(PathBuf, String)>::new();
    let mut errors = 0usize;

    for page_index in 0..pages {
        match doc.extract_tables(page_index) {
            Ok(tables) => {
                let mut candidates: Vec<TableCandidate> = merge_single_row_tables(
                    tables
                        .into_iter()
                        .enumerate()
                        .map(|(table_index, table)| TableCandidate {
                            page_index,
                            page_number: page_index + 1,
                            table_index,
                            table: SerializableTable::from_pdf_table(&table),
                        })
                        .collect(),
                );

                for candidate in &mut candidates {
                    candidate.table.refresh_stats();
                }

                for candidate in candidates {
                    let filter_reason = table_filter_reason(&candidate.table);
                    let markdown = candidate.table.to_markdown();
                    let (status, path, bytes, filter_reason) = if let Some(reason) = filter_reason {
                        ("filtered".to_string(), None, Some(markdown.len() as u64), Some(reason))
                    } else {
                        let filename = format!(
                            "page-{:04}-table-{:03}.md",
                            candidate.page_number,
                            candidate.table_index + 1
                        );
                        let target = dir.join(filename);
                        reject_existing_path(&target, "table artifact")?;
                        output_files.push((target.clone(), markdown.clone()));
                        (
                            "written".to_string(),
                            Some(target.display().to_string()),
                            Some(markdown.len() as u64),
                            None,
                        )
                    };
                    entries.push(TableManifestEntry {
                        page_index: candidate.page_index,
                        page_number: candidate.page_number,
                        table_index: candidate.table_index,
                        path,
                        status,
                        filter_reason,
                        bytes,
                        table: candidate.table,
                    });
                }
            }
            Err(err) => {
                errors += 1;
                entries.push(TableManifestEntry {
                    page_index,
                    page_number: page_index + 1,
                    table_index: 0,
                    path: None,
                    status: "error".to_string(),
                    filter_reason: Some(format!("extract_page: {err}")),
                    bytes: None,
                    table: SerializableTable::empty(),
                });
            }
        }
    }

    let written = entries.iter().filter(|entry| entry.status == "written").count();
    let filtered = entries
        .iter()
        .filter(|entry| entry.status == "filtered")
        .count();
    let status = artifact_status(entries.len(), written, filtered, errors);
    let manifest = TableManifest {
        kind: "tables",
        status: &status,
        pages,
        total_candidates: entries.len(),
        written,
        filtered,
        errors,
        entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| GrokSearchError::Parse(format!("serialize tables manifest: {err}")))?;
    let mut total_bytes = manifest_bytes.len() as u64;

    for (path, markdown) in output_files {
        total_bytes += markdown.len() as u64;
        std::fs::write(&path, markdown)
            .map_err(|err| GrokSearchError::Io(format!("write table artifact {}: {err}", path.display())))?;
    }
    std::fs::write(&manifest_path, manifest_bytes).map_err(|err| {
        GrokSearchError::Io(format!(
            "write tables manifest {}: {err}",
            manifest_path.display()
        ))
    })?;

    Ok(AcademicParseArtifact {
        kind: "tables".to_string(),
        status,
        path: Some(manifest_path.display().to_string()),
        count: Some(written),
        bytes: Some(total_bytes),
        reason: artifact_reason(filtered, errors),
    })
}

fn require_output_dir(kind: &str, dir: Option<&str>) -> Result<PathBuf> {
    let dir = dir.ok_or_else(|| {
        GrokSearchError::InvalidParams(format!(
            "{kind}_dir is required when extract_{kind}=true"
        ))
    })?;
    let dir = PathBuf::from(dir);
    ensure_output_dir(&dir, kind)?;
    Ok(dir)
}

fn artifact_status(total: usize, written: usize, filtered: usize, errors: usize) -> String {
    if written > 0 && errors > 0 {
        "partial"
    } else if written > 0 {
        "written"
    } else if total == 0 {
        "empty"
    } else if filtered > 0 && errors == 0 {
        "filtered"
    } else {
        "partial"
    }
    .to_string()
}

fn artifact_reason(filtered: usize, errors: usize) -> Option<String> {
    match (filtered, errors) {
        (0, 0) => None,
        (_, 0) => Some(format!("filtered {filtered} candidates")),
        (0, _) => Some(format!("{errors} page or candidate errors")),
        _ => Some(format!("filtered {filtered} candidates; {errors} page or candidate errors")),
    }
}

pub(crate) fn image_filter_reason(width: u32, height: u32, bbox: Option<Rect>) -> Option<String> {
    let pixel_area = u64::from(width) * u64::from(height);
    if pixel_area < IMAGE_MIN_PIXEL_AREA {
        return Some("tiny_pixel".to_string());
    }
    if let Some(rect) = bbox {
        let area = rect.width.abs() * rect.height.abs();
        let large_area = area >= IMAGE_MIN_BBOX_AREA;
        let large_shape = rect.width.abs() >= IMAGE_MIN_BBOX_WIDTH
            && rect.height.abs() >= IMAGE_MIN_BBOX_HEIGHT;
        if !large_area && !large_shape {
            return Some("small_bbox".to_string());
        }
    }
    None
}

fn table_filter_reason(table: &SerializableTable) -> Option<String> {
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
    None
}

fn bbox_area(bbox: Option<Rect>) -> Option<f32> {
    bbox.map(|rect| rect.width.abs() * rect.height.abs())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn merge_single_row_tables(mut tables: Vec<TableCandidate>) -> Vec<TableCandidate> {
    tables.sort_by(|left, right| {
        left.table
            .bbox
            .map(|bbox| bbox.y)
            .unwrap_or(f32::MAX)
            .partial_cmp(&right.table.bbox.map(|bbox| bbox.y).unwrap_or(f32::MAX))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.table_index.cmp(&right.table_index))
    });

    let mut merged = Vec::<TableCandidate>::new();
    for candidate in tables {
        if let Some(last) = merged.last_mut() {
            if can_merge_single_row_tables(last, &candidate) {
                last.table.rows.extend(candidate.table.rows);
                last.table.has_header |= candidate.table.has_header;
                last.table.bbox = merge_bbox(last.table.bbox, candidate.table.bbox);
                last.table.refresh_stats();
                continue;
            }
        }
        merged.push(candidate);
    }
    merged
}

fn can_merge_single_row_tables(left: &TableCandidate, right: &TableCandidate) -> bool {
    if left.page_index != right.page_index {
        return false;
    }
    if left.table.col_count != right.table.col_count || left.table.col_count < 2 {
        return false;
    }
    if left.table.rows.len() > 1 || right.table.rows.len() != 1 {
        return false;
    }
    let (Some(left_bbox), Some(right_bbox)) = (left.table.bbox, right.table.bbox) else {
        return false;
    };
    let x_close = (left_bbox.x - right_bbox.x).abs() <= 12.0;
    let width_close = (left_bbox.width - right_bbox.width).abs() <= 24.0;
    let vertical_gap = if right_bbox.y >= left_bbox.y {
        right_bbox.y - (left_bbox.y + left_bbox.height)
    } else {
        left_bbox.y - (right_bbox.y + right_bbox.height)
    };
    x_close && width_close && (-4.0..=24.0).contains(&vertical_gap)
}

fn merge_bbox(left: Option<Rect>, right: Option<Rect>) -> Option<Rect> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let x1 = left.x.min(right.x);
            let y1 = left.y.min(right.y);
            let x2 = (left.x + left.width).max(right.x + right.width);
            let y2 = (left.y + left.height).max(right.y + right.height);
            Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
        }
        (Some(rect), None) | (None, Some(rect)) => Some(rect),
        (None, None) => None,
    }
}

#[derive(Debug)]
struct ImageCandidate {
    page_index: usize,
    page_number: usize,
    image_index: usize,
    filename: String,
    png: Vec<u8>,
    width: u32,
    height: u32,
    pixel_area: u64,
    bbox: Option<Rect>,
    bbox_area: Option<f32>,
    color_space: String,
    bits_per_component: u8,
    rotation_degrees: i32,
    matrix: [f32; 6],
    sha256: String,
    filter_reason: Option<String>,
    sort_area: f32,
}

#[derive(Debug)]
struct TableCandidate {
    page_index: usize,
    page_number: usize,
    table_index: usize,
    table: SerializableTable,
}

#[derive(Debug, Serialize)]
struct ImageManifest<'a> {
    kind: &'a str,
    status: &'a str,
    pages: usize,
    total_candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    entries: Vec<ImageManifestEntry>,
}

#[derive(Debug, Serialize)]
struct ImageManifestEntry {
    page_index: usize,
    page_number: usize,
    image_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_reason: Option<String>,
    width: u32,
    height: u32,
    pixel_area: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<Rect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox_area: Option<f32>,
    color_space: String,
    bits_per_component: u8,
    rotation_degrees: i32,
    matrix: [f32; 6],
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Debug, Serialize)]
struct TableManifest<'a> {
    kind: &'a str,
    status: &'a str,
    pages: usize,
    total_candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    entries: Vec<TableManifestEntry>,
}

#[derive(Debug, Serialize)]
struct TableManifestEntry {
    page_index: usize,
    page_number: usize,
    table_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    table: SerializableTable,
}

#[derive(Debug, Clone, Serialize)]
struct SerializableTable {
    rows: Vec<SerializableTableRow>,
    has_header: bool,
    col_count: usize,
    row_count: usize,
    non_empty_cells: usize,
    total_cells: usize,
    empty_cell_ratio: f32,
    real_grid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<Rect>,
}

impl SerializableTable {
    fn empty() -> Self {
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

    fn from_pdf_table(table: &Table) -> Self {
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

    fn refresh_stats(&mut self) {
        self.row_count = self.rows.len();
        self.col_count = self
            .col_count
            .max(self.rows.iter().map(|row| row.cells.len()).max().unwrap_or(0));
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

    fn to_markdown(&self) -> String {
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
        let header = self.rows.first().map(|row| row.cells.as_slice()).unwrap_or(&[]);
        lines.push(markdown_row(header, cols));
        lines.push(format!(
            "|{}|",
            std::iter::repeat("---").take(cols).collect::<Vec<_>>().join("|")
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
        .filter(|row| row.cells.iter().filter(|cell| !cell.text.trim().is_empty()).count() >= 2)
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
struct SerializableTableRow {
    is_header: bool,
    cells: Vec<SerializableTableCell>,
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
struct SerializableTableCell {
    text: String,
    colspan: u32,
    rowspan: u32,
    is_header: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<Rect>,
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
