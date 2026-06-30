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
const IMAGE_COMPONENT_GROUP_MIN_MEMBERS: usize = 2;
const IMAGE_COMPONENT_GROUP_MAX_GAP: f32 = 36.0;
const TABLE_MAX_EMPTY_RATIO: f32 = 0.5;

pub fn extract_image_artifacts(
    doc: &PdfDocument,
    pages: usize,
    dir: Option<&str>,
    page_texts: Option<&[String]>,
) -> Result<AcademicParseArtifact> {
    let dir = require_output_dir("images", dir)?;
    let manifest_path = dir.join("images.json");
    reject_existing_path(&manifest_path, "images manifest")?;

    let mut entries = Vec::new();
    let mut page_kept = HashMap::<usize, usize>::new();
    let mut seen_hashes = HashMap::<String, String>::new();
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
                                rank: None,
                                path: None,
                                status: "error".to_string(),
                                filter_reason: Some(format!("encode_png: {err}")),
                                dedupe_of: None,
                                diagnostics: Vec::new(),
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

                for (rank, candidate) in candidates.into_iter().enumerate() {
                    let kept_on_page = page_kept.entry(candidate.page_index).or_default();
                    let mut filter_reason = candidate.filter_reason.clone();
                    let mut dedupe_of = None;
                    let mut diagnostics = Vec::new();
                    if let Some(reason) = filter_reason.as_deref() {
                        diagnostics.push(format!("size_filter:{reason}"));
                    }
                    if filter_reason.is_none() {
                        if let Some(first_path) = seen_hashes.get(&candidate.sha256) {
                            filter_reason = Some("duplicate".to_string());
                            dedupe_of = Some(first_path.clone());
                            diagnostics.push("same_sha256_as_written_candidate".to_string());
                        }
                    }
                    if filter_reason.is_none() && *kept_on_page >= IMAGE_MAX_PER_PAGE {
                        filter_reason = Some("page_limit".to_string());
                        diagnostics.push(format!("page_limit:{IMAGE_MAX_PER_PAGE}"));
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
                        let path = target.display().to_string();
                        *kept_on_page += 1;
                        seen_hashes.insert(candidate.sha256.clone(), path.clone());
                        output_files.push((target.clone(), candidate.png.clone()));
                        (
                            "written".to_string(),
                            Some(path),
                            Some((candidate.png.len()) as u64),
                        )
                    };

                    entries.push(ImageManifestEntry {
                        page_index: candidate.page_index,
                        page_number: candidate.page_number,
                        image_index: candidate.image_index,
                        rank: Some(rank + 1),
                        path,
                        status,
                        filter_reason,
                        dedupe_of,
                        diagnostics,
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
                    rank: None,
                    path: None,
                    status: "error".to_string(),
                    filter_reason: Some(format!("extract_page: {err}")),
                    dedupe_of: None,
                    diagnostics: Vec::new(),
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

    let written = entries
        .iter()
        .filter(|entry| entry.status == "written")
        .count();
    let filtered = entries
        .iter()
        .filter(|entry| entry.status == "filtered")
        .count();
    let status = artifact_status(entries.len(), written, filtered, errors);
    let page_summaries = image_page_summaries(&entries, page_texts, pages);
    let component_groups = image_component_groups(&entries);
    let caption_only_figures = caption_only_figures(page_texts, &entries, pages);
    let manifest = ImageManifest {
        kind: "images",
        status: &status,
        pipeline: image_pipeline_stages(),
        pages,
        total_candidates: entries.len(),
        written,
        filtered,
        errors,
        page_summaries,
        component_groups,
        caption_only_figures,
        entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| GrokSearchError::Parse(format!("serialize images manifest: {err}")))?;
    let mut total_bytes = manifest_bytes.len() as u64;

    for (path, bytes) in output_files {
        total_bytes += bytes.len() as u64;
        std::fs::write(&path, bytes).map_err(|err| {
            GrokSearchError::Io(format!("write image artifact {}: {err}", path.display()))
        })?;
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
    page_texts: Option<&[String]>,
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
                let profile_counts = table_profile_counts(doc, page_index, tables.len());
                let mut candidates: Vec<TableCandidate> = merge_single_row_tables(
                    tables
                        .into_iter()
                        .enumerate()
                        .map(|(table_index, table)| TableCandidate {
                            page_index,
                            page_number: page_index + 1,
                            table_index,
                            strategy: TableStrategy::PdfOxideGeometry,
                            table: SerializableTable::from_pdf_table(&table),
                        })
                        .collect(),
                );
                let next_index = candidates
                    .iter()
                    .map(|candidate| candidate.table_index)
                    .max()
                    .map(|index| index + 1)
                    .unwrap_or(0);
                for candidate in &mut candidates {
                    candidate.table.refresh_stats();
                }
                let has_usable_geometry_table = candidates.iter().any(|candidate| {
                    candidate.strategy == TableStrategy::PdfOxideGeometry
                        && table_filter_reason(&candidate.table).is_none()
                });
                if !has_usable_geometry_table {
                    if let Some(page_text) = page_texts.and_then(|texts| texts.get(page_index)) {
                        candidates.extend(markdown_table_candidates(
                            page_text,
                            page_index,
                            page_index + 1,
                            next_index,
                        ));
                    }
                }

                for candidate in candidates {
                    let filter_reason = table_filter_reason(&candidate.table);
                    let markdown = candidate.table.to_markdown();
                    let quality =
                        TableQuality::from_table(&candidate.table, filter_reason.as_deref());
                    let (status, path, bytes, filter_reason) = if let Some(reason) = filter_reason {
                        (
                            "filtered".to_string(),
                            None,
                            Some(markdown.len() as u64),
                            Some(reason),
                        )
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
                        strategy: candidate.strategy.as_str().to_string(),
                        path,
                        status,
                        filter_reason,
                        bytes,
                        quality,
                        profile_counts: profile_counts.clone(),
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
                    strategy: TableStrategy::PdfOxideGeometry.as_str().to_string(),
                    quality: TableQuality::empty(),
                    profile_counts: TableProfileCounts::error(),
                    table: SerializableTable::empty(),
                });
            }
        }
    }

    let written = entries
        .iter()
        .filter(|entry| entry.status == "written")
        .count();
    let filtered = entries
        .iter()
        .filter(|entry| entry.status == "filtered")
        .count();
    let status = artifact_status(entries.len(), written, filtered, errors);
    let page_summaries = table_page_summaries(&entries, pages);
    let manifest = TableManifest {
        kind: "tables",
        status: &status,
        pipeline: table_pipeline_stages(),
        pages,
        total_candidates: entries.len(),
        written,
        filtered,
        errors,
        page_summaries,
        entries,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| GrokSearchError::Parse(format!("serialize tables manifest: {err}")))?;
    let mut total_bytes = manifest_bytes.len() as u64;

    for (path, markdown) in output_files {
        total_bytes += markdown.len() as u64;
        std::fs::write(&path, markdown).map_err(|err| {
            GrokSearchError::Io(format!("write table artifact {}: {err}", path.display()))
        })?;
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
        GrokSearchError::InvalidParams(format!("{kind}_dir is required when extract_{kind}=true"))
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
        _ => Some(format!(
            "filtered {filtered} candidates; {errors} page or candidate errors"
        )),
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
        let large_shape =
            rect.width.abs() >= IMAGE_MIN_BBOX_WIDTH && rect.height.abs() >= IMAGE_MIN_BBOX_HEIGHT;
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
    if looks_like_text_flow_table(table) {
        return Some("text_flow_fallback".to_string());
    }
    None
}

fn table_profile_counts(
    doc: &PdfDocument,
    page_index: usize,
    default_count: usize,
) -> TableProfileCounts {
    let strict = doc
        .extract_tables_with_config(
            page_index,
            pdf_oxide::structure::TableDetectionConfig::strict(),
        )
        .map(|tables| tables.len())
        .ok();
    let relaxed = doc
        .extract_tables_with_config(
            page_index,
            pdf_oxide::structure::TableDetectionConfig::relaxed(),
        )
        .map(|tables| tables.len())
        .ok();
    TableProfileCounts {
        default: Some(default_count),
        strict,
        relaxed,
    }
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
            .then_with(|| {
                left.table
                    .bbox
                    .map(|bbox| bbox.x)
                    .unwrap_or(f32::MAX)
                    .partial_cmp(&right.table.bbox.map(|bbox| bbox.x).unwrap_or(f32::MAX))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
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
    if left.strategy != right.strategy {
        return false;
    }
    if left.table.col_count != right.table.col_count || left.table.col_count < 2 {
        return false;
    }
    if right.table.rows.len() != 1 {
        return false;
    }
    if left.table.rows.is_empty() || left.table.rows.len() > 32 {
        return false;
    }
    let (Some(left_bbox), Some(right_bbox)) = (left.table.bbox, right.table.bbox) else {
        return false;
    };
    let x_close = (left_bbox.x - right_bbox.x).abs() <= 18.0;
    let width_close = (left_bbox.width - right_bbox.width).abs() <= 36.0;
    let vertical_gap = if right_bbox.y >= left_bbox.y {
        right_bbox.y - (left_bbox.y + left_bbox.height)
    } else {
        left_bbox.y - (right_bbox.y + right_bbox.height)
    };
    x_close && width_close && (-8.0..=44.0).contains(&vertical_gap)
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

fn image_pipeline_stages() -> Vec<&'static str> {
    vec![
        "collect_pdf_oxide_image_objects",
        "encode_png_and_hash",
        "rank_by_display_area",
        "filter_dedupe_and_page_limit",
        "group_filtered_components",
        "detect_caption_only_figures",
        "write_artifacts_and_manifest",
    ]
}

fn image_page_summaries(
    entries: &[ImageManifestEntry],
    page_texts: Option<&[String]>,
    pages: usize,
) -> Vec<ImagePageSummary> {
    let mut summaries = Vec::new();
    for page_index in 0..pages {
        let page_entries = entries
            .iter()
            .filter(|entry| entry.page_index == page_index)
            .collect::<Vec<_>>();
        let captions = page_texts
            .and_then(|texts| texts.get(page_index))
            .map(|text| detect_figure_captions(text).len())
            .unwrap_or(0);
        if page_entries.is_empty() && captions == 0 {
            continue;
        }
        summaries.push(ImagePageSummary {
            page_index,
            page_number: page_index + 1,
            candidates: page_entries.len(),
            written: page_entries
                .iter()
                .filter(|entry| entry.status == "written")
                .count(),
            filtered: page_entries
                .iter()
                .filter(|entry| entry.status == "filtered")
                .count(),
            errors: page_entries
                .iter()
                .filter(|entry| entry.status == "error")
                .count(),
            figure_captions: captions,
        });
    }
    summaries
}

fn table_page_summaries(entries: &[TableManifestEntry], pages: usize) -> Vec<TablePageSummary> {
    let mut summaries = Vec::new();
    for page_index in 0..pages {
        let page_entries = entries
            .iter()
            .filter(|entry| entry.page_index == page_index)
            .collect::<Vec<_>>();
        if page_entries.is_empty() {
            continue;
        }
        let profile_counts = page_entries
            .first()
            .map(|entry| entry.profile_counts.clone())
            .unwrap_or_else(TableProfileCounts::empty);
        summaries.push(TablePageSummary {
            page_index,
            page_number: page_index + 1,
            candidates: page_entries.len(),
            written: page_entries
                .iter()
                .filter(|entry| entry.status == "written")
                .count(),
            filtered: page_entries
                .iter()
                .filter(|entry| entry.status == "filtered")
                .count(),
            errors: page_entries
                .iter()
                .filter(|entry| entry.status == "error")
                .count(),
            profile_counts,
        });
    }
    summaries
}

fn table_pipeline_stages() -> Vec<&'static str> {
    vec![
        "collect_pdf_oxide_geometry_tables",
        "merge_single_row_fragments",
        "collect_markdown_text_fallback_tables",
        "score_and_filter_candidates",
        "write_artifacts_and_manifest",
    ]
}

fn image_component_groups(entries: &[ImageManifestEntry]) -> Vec<ImageComponentGroup> {
    let mut groups = Vec::new();
    let mut by_page = HashMap::<usize, Vec<&ImageManifestEntry>>::new();
    for entry in entries {
        let Some(reason) = entry.filter_reason.as_deref() else {
            continue;
        };
        if entry.bbox.is_none() || !matches!(reason, "tiny_pixel" | "small_bbox") {
            continue;
        }
        by_page.entry(entry.page_index).or_default().push(entry);
    }

    for mut page_entries in by_page.into_values() {
        page_entries.sort_by(|left, right| {
            left.bbox
                .map(|bbox| bbox.y)
                .unwrap_or(f32::MAX)
                .partial_cmp(&right.bbox.map(|bbox| bbox.y).unwrap_or(f32::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    left.bbox
                        .map(|bbox| bbox.x)
                        .unwrap_or(f32::MAX)
                        .partial_cmp(&right.bbox.map(|bbox| bbox.x).unwrap_or(f32::MAX))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let mut current = Vec::<&ImageManifestEntry>::new();
        for entry in page_entries {
            let current_bbox = current
                .iter()
                .fold(None, |bbox, current| merge_bbox(bbox, current.bbox));
            if current_bbox.is_some_and(|bbox| component_group_bbox_is_close(bbox, entry.bbox)) {
                current.push(entry);
            } else {
                push_image_component_group(&mut groups, &mut current);
                current.push(entry);
            }
        }
        push_image_component_group(&mut groups, &mut current);
    }

    groups
}

fn component_group_bbox_is_close(group_bbox: Rect, next_bbox: Option<Rect>) -> bool {
    let Some(next_bbox) = next_bbox else {
        return false;
    };
    let group_right = group_bbox.x + group_bbox.width;
    let group_bottom = group_bbox.y + group_bbox.height;
    let next_right = next_bbox.x + next_bbox.width;
    let next_bottom = next_bbox.y + next_bbox.height;
    let horizontal_gap = if next_bbox.x > group_right {
        next_bbox.x - group_right
    } else if group_bbox.x > next_right {
        group_bbox.x - next_right
    } else {
        0.0
    };
    let vertical_gap = if next_bbox.y > group_bottom {
        next_bbox.y - group_bottom
    } else if group_bbox.y > next_bottom {
        group_bbox.y - next_bottom
    } else {
        0.0
    };
    horizontal_gap <= IMAGE_COMPONENT_GROUP_MAX_GAP && vertical_gap <= IMAGE_COMPONENT_GROUP_MAX_GAP
}

fn push_image_component_group(
    groups: &mut Vec<ImageComponentGroup>,
    current: &mut Vec<&ImageManifestEntry>,
) {
    if current.len() >= IMAGE_COMPONENT_GROUP_MIN_MEMBERS {
        let bbox = current
            .iter()
            .fold(None, |bbox, entry| merge_bbox(bbox, entry.bbox));
        let filter_reasons = current
            .iter()
            .filter_map(|entry| entry.filter_reason.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut filter_reasons = filter_reasons;
        filter_reasons.sort();
        let first = current[0];
        groups.push(ImageComponentGroup {
            page_index: first.page_index,
            page_number: first.page_number,
            candidates: current.len(),
            bbox,
            filter_reasons,
            image_indexes: current.iter().map(|entry| entry.image_index).collect(),
        });
    }
    current.clear();
}

fn caption_only_figures(
    page_texts: Option<&[String]>,
    entries: &[ImageManifestEntry],
    pages: usize,
) -> Vec<CaptionOnlyFigure> {
    let Some(page_texts) = page_texts else {
        return Vec::new();
    };
    let written_large_image_pages = entries
        .iter()
        .filter(|entry| entry.status == "written")
        .filter(|entry| entry.bbox_area.unwrap_or(0.0) >= IMAGE_MIN_BBOX_AREA)
        .map(|entry| entry.page_index)
        .collect::<HashSet<_>>();
    let mut figures = Vec::new();
    for page_index in 0..pages {
        let Some(page_text) = page_texts.get(page_index) else {
            continue;
        };
        for caption in detect_figure_captions(page_text) {
            let reason = if written_large_image_pages.contains(&page_index) {
                "figure_caption_with_unverified_bitmap_match"
            } else {
                "figure_caption_without_written_large_bitmap"
            };
            figures.push(CaptionOnlyFigure {
                page_index,
                page_number: page_index + 1,
                caption,
                reason: reason.to_string(),
            });
        }
    }
    figures
}

fn detect_figure_captions(page_text: &str) -> Vec<String> {
    page_text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let lower = trimmed.to_ascii_lowercase();
            (lower.starts_with("fig.") || lower.starts_with("figure ")).then(|| trimmed.to_string())
        })
        .take(16)
        .collect()
}

fn markdown_table_candidates(
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

fn looks_like_text_flow_table(table: &SerializableTable) -> bool {
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
    strategy: TableStrategy,
    table: SerializableTable,
}

#[derive(Debug, Serialize)]
struct ImageManifest<'a> {
    kind: &'a str,
    status: &'a str,
    pipeline: Vec<&'a str>,
    pages: usize,
    total_candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    page_summaries: Vec<ImagePageSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    component_groups: Vec<ImageComponentGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    caption_only_figures: Vec<CaptionOnlyFigure>,
    entries: Vec<ImageManifestEntry>,
}

#[derive(Debug, Serialize)]
struct ImagePageSummary {
    page_index: usize,
    page_number: usize,
    candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    figure_captions: usize,
}

#[derive(Debug, Serialize)]
struct ImageComponentGroup {
    page_index: usize,
    page_number: usize,
    candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    bbox: Option<Rect>,
    filter_reasons: Vec<String>,
    image_indexes: Vec<usize>,
}

#[derive(Debug, Serialize)]
struct CaptionOnlyFigure {
    page_index: usize,
    page_number: usize,
    caption: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize)]
struct ImageManifestEntry {
    page_index: usize,
    page_number: usize,
    image_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dedupe_of: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<String>,
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
    pipeline: Vec<&'a str>,
    pages: usize,
    total_candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    page_summaries: Vec<TablePageSummary>,
    entries: Vec<TableManifestEntry>,
}

#[derive(Debug, Serialize)]
struct TablePageSummary {
    page_index: usize,
    page_number: usize,
    candidates: usize,
    written: usize,
    filtered: usize,
    errors: usize,
    profile_counts: TableProfileCounts,
}

#[derive(Debug, Clone, Serialize)]
struct TableProfileCounts {
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relaxed: Option<usize>,
}

impl TableProfileCounts {
    fn empty() -> Self {
        Self {
            default: None,
            strict: None,
            relaxed: None,
        }
    }

    fn error() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Serialize)]
struct TableManifestEntry {
    page_index: usize,
    page_number: usize,
    table_index: usize,
    strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    quality: TableQuality,
    profile_counts: TableProfileCounts,
    table: SerializableTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableStrategy {
    PdfOxideGeometry,
    MarkdownTextFallback,
}

impl TableStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::PdfOxideGeometry => "pdf_oxide_geometry",
            Self::MarkdownTextFallback => "markdown_text_fallback",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct TableQuality {
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
    fn empty() -> Self {
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

    fn from_table(table: &SerializableTable, filter_reason: Option<&str>) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_consecutive_single_row_table_fragments() {
        let rows = ["A", "B", "C", "D", "E"]
            .into_iter()
            .enumerate()
            .map(|(index, label)| single_row_candidate(index, label, 100.0 + index as f32 * 18.0))
            .collect::<Vec<_>>();

        let merged = merge_single_row_tables(rows);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].table.rows.len(), 5);
        assert_eq!(merged[0].table.row_count, 5);
        assert_eq!(merged[0].table.col_count, 3);
        assert!(merged[0].table.real_grid);
    }

    #[test]
    fn detects_caption_only_figures_when_no_image_written_on_page() {
        let entries = vec![filtered_image_entry(0, Rect::new(72.0, 100.0, 10.0, 10.0))];
        let page_texts = vec!["Intro\nFig. 1: Model overview\nBody".to_string()];

        let captions = caption_only_figures(Some(&page_texts), &entries, 1);

        assert_eq!(captions.len(), 1);
        assert_eq!(captions[0].caption, "Fig. 1: Model overview");
        assert_eq!(
            captions[0].reason,
            "figure_caption_without_written_large_bitmap"
        );
    }

    #[test]
    fn groups_filtered_image_components_on_same_page() {
        let entries = vec![
            filtered_image_entry(0, Rect::new(10.0, 10.0, 12.0, 12.0)),
            filtered_image_entry(1, Rect::new(28.0, 12.0, 12.0, 12.0)),
        ];

        let groups = image_component_groups(&entries);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].candidates, 2);
        assert_eq!(groups[0].image_indexes, vec![0, 1]);
    }

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

    #[test]
    fn table_quality_records_filter_reason() {
        let table = SerializableTable::empty();
        let quality = TableQuality::from_table(&table, Some("too_few_columns"));

        assert_eq!(quality.score, 0.0);
        assert_eq!(quality.filter_reason.as_deref(), Some("too_few_columns"));
    }

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

    fn single_row_candidate(index: usize, label: &str, y: f32) -> TableCandidate {
        let mut table = SerializableTable {
            rows: vec![SerializableTableRow {
                is_header: index == 0,
                cells: ["c1", "c2", "c3"]
                    .into_iter()
                    .map(|suffix| SerializableTableCell {
                        text: format!("{label}-{suffix}"),
                        colspan: 1,
                        rowspan: 1,
                        is_header: index == 0,
                        bbox: None,
                    })
                    .collect(),
            }],
            has_header: index == 0,
            col_count: 3,
            row_count: 0,
            non_empty_cells: 0,
            total_cells: 0,
            empty_cell_ratio: 0.0,
            real_grid: false,
            bbox: Some(Rect::new(72.0, y, 420.0, 12.0)),
        };
        table.refresh_stats();
        TableCandidate {
            page_index: 0,
            page_number: 1,
            table_index: index,
            strategy: TableStrategy::PdfOxideGeometry,
            table,
        }
    }

    fn filtered_image_entry(image_index: usize, bbox: Rect) -> ImageManifestEntry {
        ImageManifestEntry {
            page_index: 0,
            page_number: 1,
            image_index,
            rank: Some(image_index + 1),
            path: None,
            status: "filtered".to_string(),
            filter_reason: Some("small_bbox".to_string()),
            dedupe_of: None,
            diagnostics: Vec::new(),
            width: 20,
            height: 20,
            pixel_area: 400,
            bbox: Some(bbox),
            bbox_area: Some(bbox.width.abs() * bbox.height.abs()),
            color_space: "DeviceRGB".to_string(),
            bits_per_component: 8,
            rotation_degrees: 0,
            matrix: [0.0; 6],
            bytes: Some(10),
            sha256: Some(format!("hash-{image_index}")),
        }
    }
}
