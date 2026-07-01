mod filters;
mod images;
mod manifest;
mod markdown_table;
mod quality;
mod tables;

use std::path::PathBuf;

use grok_search_content::ensure_output_dir;
use grok_search_types::{GrokSearchError, Result};
use pdf_oxide::geometry::Rect;
use sha2::{Digest, Sha256};

pub use images::extract_image_artifacts;
pub use tables::extract_table_artifacts;

#[cfg(test)]
pub(crate) use filters::image_filter_reason;

const IMAGE_MIN_PIXEL_AREA: u64 = 50_000;
const IMAGE_MIN_BBOX_AREA: f32 = 3_000.0;
const IMAGE_MIN_BBOX_WIDTH: f32 = 60.0;
const IMAGE_MIN_BBOX_HEIGHT: f32 = 40.0;
const IMAGE_MAX_PER_PAGE: usize = 10;
const IMAGE_COMPONENT_GROUP_MIN_MEMBERS: usize = 2;
const IMAGE_COMPONENT_GROUP_MAX_GAP: f32 = 36.0;
const TABLE_MAX_EMPTY_RATIO: f32 = 0.5;

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
