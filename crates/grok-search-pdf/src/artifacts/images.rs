use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use grok_search_content::reject_existing_path;
use grok_search_types::{AcademicParseArtifact, GrokSearchError, Result};
use pdf_oxide::geometry::Rect;
use pdf_oxide::PdfDocument;

use super::filters::image_filter_reason;
use super::manifest::{
    CaptionOnlyFigure, ImageCandidate, ImageComponentGroup, ImageManifest, ImageManifestEntry,
    ImagePageSummary,
};
use super::{
    artifact_reason, artifact_status, bbox_area, merge_bbox, require_output_dir, sha256_hex,
    IMAGE_COMPONENT_GROUP_MAX_GAP, IMAGE_COMPONENT_GROUP_MIN_MEMBERS, IMAGE_MAX_PER_PAGE,
    IMAGE_MIN_BBOX_AREA,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
