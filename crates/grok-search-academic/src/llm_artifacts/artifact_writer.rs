use std::path::{Path, PathBuf};

use grok_search_pdf::{ParsedPdfDetails, PdfVisionRenderOutcome};
use grok_search_types::{
    AcademicParseArtifact, AcademicPdfArtifactsInput, AcademicPdfVisionArtifacts, GrokSearchError,
    Result,
};

use super::validate_bbox;

pub(crate) fn write_refined_completion_artifacts(
    parsed: &ParsedPdfDetails,
    input: &AcademicPdfArtifactsInput,
    vision: &mut AcademicPdfVisionArtifacts,
) -> Result<Vec<AcademicParseArtifact>> {
    let mut artifacts = Vec::new();
    let Some(render) = parsed.vision_render.as_ref() else {
        vision
            .warnings
            .push("cannot write refined completion crops without rendered pages".to_string());
        return Ok(artifacts);
    };
    if let Some(images_dir) = input.images_dir.as_deref() {
        let mut count = 0usize;
        for completion in &mut vision.figure_completions {
            if completion.refined_bbox_norm.is_empty() || completion.status == "not_visible" {
                continue;
            }
            match write_completion_crop(
                render,
                completion.page,
                &completion.refined_bbox_norm,
                images_dir,
                "figure",
                completion.label.as_deref(),
            ) {
                Ok(path) => {
                    completion.crop_path = Some(path);
                    count += 1;
                }
                Err(err) => completion
                    .warnings
                    .push(format!("write figure crop failed: {err}")),
            }
        }
        if count > 0 {
            artifacts.push(AcademicParseArtifact {
                kind: "llm_vision_refined_figures".to_string(),
                status: "ok".to_string(),
                path: Some(images_dir.to_string()),
                count: Some(count),
                bytes: None,
                reason: Some("source=llm_vision_refined".to_string()),
            });
        }
    }
    if let Some(tables_dir) = input.tables_dir.as_deref() {
        let mut markdown_count = 0usize;
        let mut crop_count = 0usize;
        for completion in &mut vision.table_completions {
            if let Some(markdown) = completion.markdown.as_deref() {
                match write_completion_markdown(
                    tables_dir,
                    completion.page,
                    completion.label.as_deref(),
                    markdown,
                ) {
                    Ok(path) => {
                        completion.markdown_path = Some(path);
                        markdown_count += 1;
                    }
                    Err(err) => completion
                        .warnings
                        .push(format!("write table markdown failed: {err}")),
                }
            }
            if !completion.refined_bbox_norm.is_empty() && completion.status != "not_visible" {
                match write_completion_crop(
                    render,
                    completion.page,
                    &completion.refined_bbox_norm,
                    tables_dir,
                    "table",
                    completion.label.as_deref(),
                ) {
                    Ok(path) => {
                        completion.crop_path = Some(path);
                        crop_count += 1;
                    }
                    Err(err) => completion
                        .warnings
                        .push(format!("write table crop failed: {err}")),
                }
            }
        }
        if markdown_count > 0 || crop_count > 0 {
            artifacts.push(AcademicParseArtifact {
                kind: "llm_vision_refined_tables".to_string(),
                status: "ok".to_string(),
                path: Some(tables_dir.to_string()),
                count: Some(markdown_count),
                bytes: None,
                reason: Some(format!(
                    "source=llm_vision_refined; crops={crop_count}; markdown={markdown_count}"
                )),
            });
        }
    }
    Ok(artifacts)
}

pub(super) fn write_vision_artifact(
    dir: &str,
    output: &AcademicPdfVisionArtifacts,
) -> Result<String> {
    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).map_err(|err| {
        GrokSearchError::Io(format!(
            "create vision artifact dir {}: {err}",
            dir.display()
        ))
    })?;
    let path = dir.join("vision.json");
    if path.exists() {
        return Err(GrokSearchError::InvalidParams(format!(
            "vision artifact path already exists: {}",
            path.display()
        )));
    }
    let bytes = serde_json::to_vec_pretty(output)
        .map_err(|err| GrokSearchError::Parse(format!("serialize vision artifact: {err}")))?;
    std::fs::write(&path, bytes)
        .map_err(|err| GrokSearchError::Io(format!("write vision artifact: {err}")))?;
    Ok(path.display().to_string())
}

fn write_completion_crop(
    render: &PdfVisionRenderOutcome,
    page: usize,
    bbox: &[f32],
    dir: &str,
    kind: &str,
    label: Option<&str>,
) -> Result<String> {
    let rendered = render
        .pages
        .iter()
        .find(|rendered| rendered.page_number == page)
        .ok_or_else(|| GrokSearchError::NotFound(format!("rendered page {page} not found")))?;
    let bbox = validate_bbox(bbox).ok_or_else(|| {
        GrokSearchError::InvalidParams(format!("invalid refined {kind} bbox on page {page}"))
    })?;
    let image = image::load_from_memory(&rendered.png)
        .map_err(|err| GrokSearchError::Parse(format!("decode rendered page PNG: {err}")))?;
    let width = image.width();
    let height = image.height();
    let x0 = ((bbox[0] * width as f32).round() as u32).min(width.saturating_sub(1));
    let y0 = ((bbox[1] * height as f32).round() as u32).min(height.saturating_sub(1));
    let x1 = ((bbox[2] * width as f32).round() as u32)
        .min(width)
        .max(x0 + 1);
    let y1 = ((bbox[3] * height as f32).round() as u32)
        .min(height)
        .max(y0 + 1);
    let cropped = image.crop_imm(x0, y0, x1 - x0, y1 - y0);
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).map_err(|err| {
        GrokSearchError::Io(format!(
            "create refined {kind} artifact dir {}: {err}",
            dir.display()
        ))
    })?;
    let path = unique_completion_path(&dir, page, kind, label, "png");
    cropped
        .save(&path)
        .map_err(|err| GrokSearchError::Io(format!("write refined {kind} crop: {err}")))?;
    Ok(path.display().to_string())
}

fn write_completion_markdown(
    dir: &str,
    page: usize,
    label: Option<&str>,
    markdown: &str,
) -> Result<String> {
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).map_err(|err| {
        GrokSearchError::Io(format!(
            "create refined table artifact dir {}: {err}",
            dir.display()
        ))
    })?;
    let path = unique_completion_path(&dir, page, "table", label, "md");
    std::fs::write(&path, markdown).map_err(|err| {
        GrokSearchError::Io(format!(
            "write refined table markdown {}: {err}",
            path.display()
        ))
    })?;
    Ok(path.display().to_string())
}

fn unique_completion_path(
    dir: &Path,
    page: usize,
    kind: &str,
    label: Option<&str>,
    extension: &str,
) -> PathBuf {
    let label = label
        .map(sanitize_label)
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| kind.to_string());
    for index in 1..=999usize {
        let path = dir.join(format!(
            "llm_refined_p{page:04}_{kind}_{index:02}_{label}.{extension}"
        ));
        if !path.exists() {
            return path;
        }
    }
    dir.join(format!(
        "llm_refined_p{page:04}_{kind}_{}_{}.{}",
        uuid::Uuid::new_v4(),
        label,
        extension
    ))
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(80)
        .collect()
}
