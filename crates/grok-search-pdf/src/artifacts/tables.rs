use std::path::PathBuf;

use grok_search_content::reject_existing_path;
use grok_search_types::{AcademicParseArtifact, GrokSearchError, Result};
use pdf_oxide::PdfDocument;

use super::filters::table_filter_reason;
use super::manifest::{
    SerializableTable, TableCandidate, TableManifest, TableManifestEntry, TablePageSummary,
    TableProfileCounts, TableStrategy,
};
use super::markdown_table::markdown_table_candidates;
use super::quality::TableQuality;
use super::{artifact_reason, artifact_status, merge_bbox, require_output_dir};

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

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_oxide::geometry::Rect;

    use crate::artifacts::manifest::{SerializableTableCell, SerializableTableRow};

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
}
