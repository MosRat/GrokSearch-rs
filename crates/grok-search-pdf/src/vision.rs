use grok_search_types::{GrokSearchError, Result};
use pdf_oxide::PdfDocument;
use serde::{Deserialize, Serialize};

use crate::text::analyze_text_signals;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfVisionSourceBundle {
    pub pdf_sha256: String,
    pub pages: Vec<PdfVisionPage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfVisionPage {
    pub page_index: usize,
    pub page_number: usize,
    pub markdown: String,
    pub plain_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<PdfVisionAnchor>,
    pub figure_caption_count: usize,
    pub table_caption_count: usize,
    pub url_count: usize,
    pub image_count: usize,
    pub large_image_count: usize,
    pub tiny_image_count: usize,
    pub table_count: usize,
    pub triage_priority: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triage_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfVisionAnchor {
    pub anchor_id: String,
    pub page: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub char_start: usize,
    pub char_end: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfRenderedPage {
    pub page_index: usize,
    pub page_number: usize,
    pub dpi: u16,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfVisionRenderFailure {
    pub page_index: usize,
    pub page_number: usize,
    pub warning: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfVisionRenderOutcome {
    pub pages: Vec<PdfRenderedPage>,
    pub failures: Vec<PdfVisionRenderFailure>,
}

pub fn build_vision_source_bundle(
    doc: &PdfDocument,
    pages: usize,
    markdown_pages: &[String],
    plain_pages: &[String],
    pdf_sha256: String,
) -> PdfVisionSourceBundle {
    let mut records = Vec::with_capacity(pages);
    let mut warnings = Vec::new();
    for page_index in 0..pages {
        let markdown = markdown_pages.get(page_index).cloned().unwrap_or_default();
        let plain_text = plain_pages
            .get(page_index)
            .cloned()
            .unwrap_or_else(|| markdown.clone());
        let signals = analyze_text_signals(&markdown);
        let (image_count, large_image_count, tiny_image_count) =
            image_signal_counts(doc, page_index, &mut warnings);
        let table_count = table_signal_count(doc, page_index, &mut warnings);
        let (triage_priority, triage_reasons) = triage_page(
            signals.figure_caption_count,
            signals.table_caption_count,
            image_count,
            large_image_count,
            tiny_image_count,
            table_count,
        );
        records.push(PdfVisionPage {
            page_index,
            page_number: page_index + 1,
            anchors: build_page_anchors(page_index + 1, &markdown),
            markdown,
            plain_text,
            figure_caption_count: signals.figure_caption_count,
            table_caption_count: signals.table_caption_count,
            url_count: signals.url_count,
            image_count,
            large_image_count,
            tiny_image_count,
            table_count,
            triage_priority,
            triage_reasons,
        });
    }
    PdfVisionSourceBundle {
        pdf_sha256,
        pages: records,
        warnings,
    }
}

pub fn select_vision_pages(bundle: &PdfVisionSourceBundle, max_pages: usize) -> Vec<PdfVisionPage> {
    let mut pages = bundle
        .pages
        .iter()
        .filter(|page| page.triage_priority > 0)
        .cloned()
        .collect::<Vec<_>>();
    pages.sort_by(|left, right| {
        right
            .triage_priority
            .cmp(&left.triage_priority)
            .then_with(|| left.page_number.cmp(&right.page_number))
    });
    pages.truncate(max_pages);
    pages
}

fn image_signal_counts(
    doc: &PdfDocument,
    page_index: usize,
    warnings: &mut Vec<String>,
) -> (usize, usize, usize) {
    let images = match doc.extract_images(page_index) {
        Ok(images) => images,
        Err(err) => {
            warnings.push(format!(
                "vision image signal extraction failed on page {}: {err}",
                page_index + 1
            ));
            return (0, 0, 0);
        }
    };
    let mut large = 0usize;
    let mut tiny = 0usize;
    for image in &images {
        let pixel_area = u64::from(image.width()) * u64::from(image.height());
        let bbox_area = image
            .bbox()
            .map(|bbox| bbox.width.abs() * bbox.height.abs());
        let geometric_area = bbox_area.unwrap_or(pixel_area as f32);
        if pixel_area >= 20_000 || geometric_area >= 12_000.0 {
            large += 1;
        }
        if pixel_area < 4_096 || geometric_area < 2_500.0 {
            tiny += 1;
        }
    }
    (images.len(), large, tiny)
}

fn table_signal_count(doc: &PdfDocument, page_index: usize, warnings: &mut Vec<String>) -> usize {
    match doc.extract_tables(page_index) {
        Ok(tables) => tables.len(),
        Err(err) => {
            warnings.push(format!(
                "vision table signal extraction failed on page {}: {err}",
                page_index + 1
            ));
            0
        }
    }
}

fn triage_page(
    figure_caption_count: usize,
    table_caption_count: usize,
    image_count: usize,
    large_image_count: usize,
    tiny_image_count: usize,
    table_count: usize,
) -> (u32, Vec<String>) {
    let mut priority = 0u32;
    let mut reasons = Vec::new();
    if figure_caption_count > 0 && large_image_count == 0 {
        priority += 80;
        reasons.push("figure_caption_without_large_bitmap".to_string());
    }
    if table_caption_count > 0 && table_count == 0 {
        priority += 65;
        reasons.push("table_caption_without_geometry_table".to_string());
    }
    if table_caption_count > 0 && table_count > 0 {
        priority += 55;
        reasons.push("table_geometry_needs_visual_check".to_string());
    }
    if tiny_image_count >= 3 && large_image_count == 0 {
        priority += 70;
        reasons.push("fragmented_image_components".to_string());
    } else if tiny_image_count >= 5 {
        priority += 45;
        reasons.push("many_tiny_image_xobjects".to_string());
    }
    if figure_caption_count > 0 && large_image_count > 0 {
        priority += 25;
        reasons.push("figure_caption_with_bitmap_verify".to_string());
    }
    if priority == 0 && table_count > 0 {
        priority = 1;
        reasons.push("baseline_table_sample".to_string());
    }
    if priority == 0 && image_count > 0 && figure_caption_count > 0 {
        priority = 1;
        reasons.push("baseline_figure_sample".to_string());
    }
    (priority, reasons)
}

fn build_page_anchors(page_number: usize, text: &str) -> Vec<PdfVisionAnchor> {
    let mut anchors = Vec::new();
    let mut char_cursor = 0usize;
    for (line_index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let line_start = char_cursor;
        let line_end = char_cursor.saturating_add(line.len());
        char_cursor = line_end.saturating_add(1);
        if trimmed.is_empty() || trimmed.starts_with("[[page:") {
            continue;
        }
        anchors.push(PdfVisionAnchor {
            anchor_id: format!("p{page_number:04}_l{:04}", line_index + 1),
            page: page_number,
            line_start: line_index + 1,
            line_end: line_index + 1,
            char_start: line_start,
            char_end: line_end,
            excerpt: trimmed.chars().take(220).collect(),
        });
        if anchors.len() >= 48 {
            break;
        }
    }
    anchors
}

pub struct PdfOxidePageRenderer;

impl PdfOxidePageRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn render_pages(
        &self,
        doc: &PdfDocument,
        pages: &[PdfVisionPage],
        dpi: u16,
    ) -> Result<PdfVisionRenderOutcome> {
        let mut rendered = Vec::new();
        let mut failures = Vec::new();
        for page in pages {
            match render_one_page(doc, page, dpi) {
                Ok(output) => rendered.push(output),
                Err(err) => failures.push(PdfVisionRenderFailure {
                    page_index: page.page_index,
                    page_number: page.page_number,
                    warning: err.to_string(),
                }),
            }
        }
        Ok(PdfVisionRenderOutcome {
            pages: rendered,
            failures,
        })
    }
}

fn render_one_page(doc: &PdfDocument, page: &PdfVisionPage, dpi: u16) -> Result<PdfRenderedPage> {
    let mut options = pdf_oxide::rendering::RenderOptions::with_dpi(u32::from(dpi));
    options.format = pdf_oxide::rendering::ImageFormat::Png;
    let image =
        pdf_oxide::rendering::render_page(doc, page.page_index, &options).map_err(|err| {
            GrokSearchError::Parse(format!("pdf_oxide render page {}: {err}", page.page_number))
        })?;
    Ok(PdfRenderedPage {
        page_index: page.page_index,
        page_number: page.page_number,
        dpi,
        width: image.width,
        height: image.height,
        png: image.data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_prioritizes_caption_without_large_bitmap() {
        let (priority, reasons) = triage_page(1, 0, 2, 0, 3, 0);
        assert!(priority >= 80);
        assert!(reasons.contains(&"figure_caption_without_large_bitmap".to_string()));
        assert!(reasons.contains(&"fragmented_image_components".to_string()));
    }

    #[test]
    fn triage_samples_geometry_tables_even_without_caption() {
        let (priority, reasons) = triage_page(0, 0, 0, 0, 0, 1);
        assert_eq!(priority, 1);
        assert_eq!(reasons, vec!["baseline_table_sample".to_string()]);
    }

    #[test]
    fn select_vision_pages_orders_by_priority_then_page_number() {
        let mut pages = Vec::new();
        for (page_number, priority) in [(3, 10), (1, 30), (2, 30), (4, 0)] {
            pages.push(PdfVisionPage {
                page_index: page_number - 1,
                page_number,
                markdown: String::new(),
                plain_text: String::new(),
                anchors: Vec::new(),
                figure_caption_count: 0,
                table_caption_count: 0,
                url_count: 0,
                image_count: 0,
                large_image_count: 0,
                tiny_image_count: 0,
                table_count: 0,
                triage_priority: priority,
                triage_reasons: vec![format!("p{priority}")],
            });
        }
        let selected = select_vision_pages(
            &PdfVisionSourceBundle {
                pdf_sha256: "pdf".to_string(),
                pages,
                warnings: Vec::new(),
            },
            2,
        );
        assert_eq!(
            selected
                .iter()
                .map(|page| page.page_number)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
}
