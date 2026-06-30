use std::io::Write;
use std::path::{Path, PathBuf};

use grok_search_content::{
    ensure_output_dir, truncate_content, write_text_file_no_overwrite, ParsedContent,
};
use grok_search_types::{
    AcademicParseArtifact, AcademicParseCapabilities, AcademicParseOptions, AcademicPdfPassReport,
    AcademicPdfProcessingReport, GrokSearchError, Result,
};

use crate::artifacts;
use crate::text::{analyze_text_signals, clean_text, TextProcessingMode};

#[derive(Debug)]
pub struct ParsedPdfDetails {
    pub pdf_sha256: String,
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    pub raw_content: Option<String>,
    pub raw_original_length: Option<usize>,
    pub raw_truncated: Option<bool>,
    pub artifacts: Vec<AcademicParseArtifact>,
    pub capabilities: AcademicParseCapabilities,
    pub processing: AcademicPdfProcessingReport,
    pub progressive_source: Option<PdfProgressiveSourceBundle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfProgressiveSourceBundle {
    pub input_profile: String,
    pub text: String,
    pub raw_markdown: String,
    pub raw_plain_text: String,
    pub reference_tail: String,
    pub pages: Vec<PdfProgressivePage>,
    pub text_sha256: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfProgressivePage {
    pub page: usize,
    pub markdown: String,
    pub plain_text: String,
    pub char_start: usize,
    pub char_end: usize,
    pub figure_caption_count: usize,
    pub table_caption_count: usize,
    pub url_count: usize,
}

pub fn parse_pdf_bytes(
    bytes: &[u8],
    format: &str,
    max_chars: Option<usize>,
) -> Result<ParsedContent> {
    let details = parse_pdf_bytes_detailed(bytes, format, max_chars, None)?;
    Ok(ParsedContent {
        content: details.content,
        original_length: details.original_length,
        truncated: details.truncated,
    })
}

pub fn parse_pdf_bytes_detailed(
    bytes: &[u8],
    format: &str,
    max_chars: Option<usize>,
    options: Option<&AcademicParseOptions>,
) -> Result<ParsedPdfDetails> {
    let pipeline_options = PdfPipelineOptions::from_parse_options(format, max_chars, options)?;
    run_pdf_pipeline(bytes, pipeline_options)
}

#[derive(Debug, Clone)]
struct PdfPipelineOptions<'a> {
    format: &'a str,
    max_chars: Option<usize>,
    parse_options: Option<&'a AcademicParseOptions>,
    text_processing_mode: TextProcessingMode,
    include_raw_content: bool,
    build_progressive_source: bool,
}

impl<'a> PdfPipelineOptions<'a> {
    fn from_parse_options(
        format: &'a str,
        max_chars: Option<usize>,
        parse_options: Option<&'a AcademicParseOptions>,
    ) -> Result<Self> {
        let text_processing_mode = TextProcessingMode::parse(
            parse_options.and_then(|options| options.text_processing_mode.as_deref()),
        )?;
        let include_raw_content = parse_options
            .and_then(|options| options.include_raw_content)
            .unwrap_or(false);
        let build_progressive_source = parse_options
            .and_then(|options| options.llm_progressive.as_ref())
            .and_then(|options| options.enabled)
            .unwrap_or(false);
        Ok(Self {
            format,
            max_chars,
            parse_options,
            text_processing_mode,
            include_raw_content,
            build_progressive_source,
        })
    }
}

fn run_pdf_pipeline(bytes: &[u8], options: PdfPipelineOptions<'_>) -> Result<ParsedPdfDetails> {
    let mut passes = Vec::new();
    let pdf_sha256 = sha256_hex(bytes);
    validate_parse_artifact_options(options.parse_options)?;
    passes.push(pass_report(
        "validate_options",
        "ok",
        None,
        None,
        Vec::new(),
    ));

    let mut file = tempfile::NamedTempFile::new()
        .map_err(|err| GrokSearchError::Io(format!("create temp PDF: {err}")))?;
    file.write_all(bytes)
        .map_err(|err| GrokSearchError::Io(format!("write temp PDF: {err}")))?;
    let path = file.path().to_path_buf();
    let doc = open_pdf_oxide_document(&path)?;
    let pages = doc
        .page_count()
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide page_count: {err}")))?;
    passes.push(pass_report("open_pdf", "ok", None, None, Vec::new()));

    let raw_pages = extract_page_texts_with_pdf_oxide(&doc, pages, options.format)?;
    let progressive_source = if options.build_progressive_source {
        Some(build_progressive_source_bundle(&doc, pages, &raw_pages)?)
    } else {
        None
    };
    let raw_content = join_page_texts(&raw_pages);
    let raw_original_length = raw_content.chars().count();
    passes.push(pass_report(
        "raw_text_extraction",
        "ok",
        None,
        Some(raw_original_length),
        Vec::new(),
    ));

    let signals = analyze_text_signals(&raw_content);
    let page_layout_signals = collect_page_layout_signals(&doc, pages);
    let mut signal_warnings = Vec::new();
    if signals.short_line_ratio >= 0.25 {
        signal_warnings.push(format!(
            "high short-line ratio {:.2}",
            signals.short_line_ratio
        ));
    }
    if signals.repeated_line_count > 0 {
        signal_warnings.push(format!(
            "{} repeated layout line candidates",
            signals.repeated_line_count
        ));
    }
    signal_warnings.extend(page_layout_signals.warnings);
    passes.push(pass_report(
        "text_signal",
        "ok",
        Some(raw_original_length),
        Some(raw_original_length),
        signal_warnings,
    ));

    let cleaned = clean_text(&raw_content, options.text_processing_mode);
    let processed_original_length = cleaned.content.chars().count();
    passes.push(pass_report(
        "text_clean",
        "ok",
        Some(raw_original_length),
        Some(processed_original_length),
        cleaned.warnings.clone(),
    ));

    let mut artifacts = Vec::new();
    let capabilities = AcademicParseCapabilities::default();
    if let Some(parse_options) = options.parse_options {
        let mut artifact_warnings = Vec::new();
        if parse_options.extract_images.unwrap_or(false) {
            artifacts.push(artifacts::extract_image_artifacts(
                &doc,
                pages,
                parse_options.images_dir.as_deref(),
                Some(&raw_pages),
            )?);
        } else if parse_options.images_dir.is_some() {
            ensure_dir_artifact("images", parse_options.images_dir.as_deref())?;
        }
        if parse_options.extract_tables.unwrap_or(false) {
            artifacts.push(artifacts::extract_table_artifacts(
                &doc,
                pages,
                parse_options.tables_dir.as_deref(),
                Some(&raw_pages),
            )?);
        } else if parse_options.tables_dir.is_some() {
            ensure_dir_artifact("tables", parse_options.tables_dir.as_deref())?;
        }
        for artifact in &artifacts {
            if let Some(reason) = artifact.reason.as_deref() {
                artifact_warnings.push(format!("{}: {reason}", artifact.kind));
            }
        }
        passes.push(pass_report(
            "artifact_extraction",
            "ok",
            None,
            None,
            artifact_warnings,
        ));
        passes.push(pass_report("artifact_refine", "ok", None, None, Vec::new()));

        let parsed = truncate_content(cleaned.content, options.max_chars);
        if let Some(path) = parse_options.save_markdown_path.as_deref() {
            artifacts.push(write_markdown_artifact(path, &parsed.content)?);
        }
        let raw_parsed_for_save = parse_options
            .save_raw_content_path
            .as_ref()
            .map(|_| truncate_content(raw_content.clone(), options.max_chars));
        let raw_parsed_for_output = if options.include_raw_content {
            Some(
                raw_parsed_for_save
                    .clone()
                    .unwrap_or_else(|| truncate_content(raw_content.clone(), options.max_chars)),
            )
        } else {
            None
        };
        if let Some(path) = parse_options.save_raw_content_path.as_deref() {
            if let Some(raw_parsed) = raw_parsed_for_save.as_ref() {
                artifacts.push(write_raw_content_artifact(path, &raw_parsed.content)?);
            }
        }
        let raw_truncated_for_report = raw_parsed_for_output
            .as_ref()
            .or(raw_parsed_for_save.as_ref())
            .map(|parsed| parsed.truncated)
            .unwrap_or(false);
        passes.push(pass_report("write_artifacts", "ok", None, None, Vec::new()));
        return Ok(finalize_details(
            parsed,
            raw_parsed_for_output,
            raw_truncated_for_report,
            raw_original_length,
            processed_original_length,
            artifacts,
            capabilities,
            options.text_processing_mode,
            passes,
            progressive_source,
            pdf_sha256,
        ));
    }

    passes.push(pass_report(
        "artifact_extraction",
        "skipped",
        None,
        None,
        Vec::new(),
    ));
    passes.push(pass_report(
        "artifact_refine",
        "skipped",
        None,
        None,
        Vec::new(),
    ));
    let parsed = truncate_content(cleaned.content, options.max_chars);
    let raw_parsed = options
        .include_raw_content
        .then(|| truncate_content(raw_content, options.max_chars));
    let raw_truncated_for_report = raw_parsed
        .as_ref()
        .map(|parsed| parsed.truncated)
        .unwrap_or(false);
    passes.push(pass_report(
        "write_artifacts",
        "skipped",
        None,
        None,
        Vec::new(),
    ));
    Ok(finalize_details(
        parsed,
        raw_parsed,
        raw_truncated_for_report,
        raw_original_length,
        processed_original_length,
        artifacts,
        capabilities,
        options.text_processing_mode,
        passes,
        progressive_source,
        pdf_sha256,
    ))
}

fn validate_parse_artifact_options(options: Option<&AcademicParseOptions>) -> Result<()> {
    let Some(options) = options else {
        return Ok(());
    };
    TextProcessingMode::parse(options.text_processing_mode.as_deref())?;
    if options.extract_images.unwrap_or(false) && options.images_dir.is_none() {
        return Err(GrokSearchError::InvalidParams(
            "images_dir is required when extract_images=true".to_string(),
        ));
    }
    if options.extract_tables.unwrap_or(false) && options.tables_dir.is_none() {
        return Err(GrokSearchError::InvalidParams(
            "tables_dir is required when extract_tables=true".to_string(),
        ));
    }
    Ok(())
}

fn finalize_details(
    parsed: ParsedContent,
    raw_parsed: Option<ParsedContent>,
    raw_truncated_for_report: bool,
    raw_original_length: usize,
    processed_original_length: usize,
    artifacts: Vec<AcademicParseArtifact>,
    capabilities: AcademicParseCapabilities,
    mode: TextProcessingMode,
    mut passes: Vec<AcademicPdfPassReport>,
    progressive_source: Option<PdfProgressiveSourceBundle>,
    pdf_sha256: String,
) -> ParsedPdfDetails {
    passes.push(pass_report(
        "finalize",
        "ok",
        Some(processed_original_length),
        Some(parsed.content.chars().count()),
        Vec::new(),
    ));
    let raw_truncated = raw_parsed.as_ref().map(|parsed| parsed.truncated);
    let raw_content = raw_parsed.map(|parsed| parsed.content);
    let warnings = passes
        .iter()
        .flat_map(|pass| pass.warnings.iter().cloned())
        .collect::<Vec<_>>();
    let processing = AcademicPdfProcessingReport {
        text_processing_mode: mode.as_str().to_string(),
        raw_original_length,
        processed_original_length,
        raw_truncated: raw_truncated_for_report,
        processed_truncated: parsed.truncated,
        passes,
        warnings,
    };
    ParsedPdfDetails {
        pdf_sha256,
        content: parsed.content,
        original_length: parsed.original_length,
        truncated: parsed.truncated,
        raw_content,
        raw_original_length: raw_content_is_requested(raw_truncated).then_some(raw_original_length),
        raw_truncated,
        artifacts,
        capabilities,
        processing,
        progressive_source,
    }
}

fn raw_content_is_requested(raw_truncated: Option<bool>) -> bool {
    raw_truncated.is_some()
}

fn pass_report(
    name: &str,
    status: &str,
    input_length: Option<usize>,
    output_length: Option<usize>,
    warnings: Vec<String>,
) -> AcademicPdfPassReport {
    AcademicPdfPassReport {
        name: name.to_string(),
        status: status.to_string(),
        input_length,
        output_length,
        warnings,
    }
}

fn open_pdf_oxide_document(path: &std::path::Path) -> Result<pdf_oxide::PdfDocument> {
    pdf_oxide::PdfDocument::open(path)
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide open: {err}")))
}

fn extract_page_texts_with_pdf_oxide(
    doc: &pdf_oxide::PdfDocument,
    pages: usize,
    format: &str,
) -> Result<Vec<String>> {
    let mut page_texts = Vec::with_capacity(pages);
    for page in 0..pages {
        let text = if format == "markdown" {
            doc.to_markdown(page, &pdf_oxide::converters::ConversionOptions::default())
        } else {
            doc.extract_text(page)
        }
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide extract page {page}: {err}")))?;
        page_texts.push(text);
    }
    Ok(page_texts)
}

fn join_page_texts(page_texts: &[String]) -> String {
    let mut out = String::new();
    for text in page_texts {
        out.push_str(text);
        out.push_str("\n\n");
    }
    out
}

fn build_progressive_source_bundle(
    doc: &pdf_oxide::PdfDocument,
    pages: usize,
    fallback_markdown_pages: &[String],
) -> Result<PdfProgressiveSourceBundle> {
    let markdown_pages =
        extract_page_texts_with_pdf_oxide(doc, pages, "markdown").unwrap_or_else(|_| {
            fallback_markdown_pages
                .iter()
                .map(ToString::to_string)
                .collect()
        });
    let plain_pages = extract_page_texts_with_pdf_oxide(doc, pages, "text").unwrap_or_else(|_| {
        fallback_markdown_pages
            .iter()
            .map(ToString::to_string)
            .collect()
    });
    let raw_markdown = join_page_texts(&markdown_pages);
    let raw_plain_text = join_page_texts(&plain_pages);
    let cleaned_markdown = clean_text(&raw_markdown, TextProcessingMode::Light).content;
    let reference_tail = reference_tail_from_plain_text(&raw_plain_text);
    let mut text = cleaned_markdown.clone();
    if !reference_tail.is_empty() && !cleaned_markdown.contains(reference_tail.trim()) {
        text.push_str("\n\n## References Plain Text Tail\n\n");
        text.push_str(&reference_tail);
    }
    let mut page_records = Vec::with_capacity(pages);
    let mut cursor = 0usize;
    for page_index in 0..pages {
        let markdown = markdown_pages.get(page_index).cloned().unwrap_or_default();
        let plain_text = plain_pages.get(page_index).cloned().unwrap_or_default();
        let signals = analyze_text_signals(&markdown);
        let char_start = cursor;
        cursor = cursor.saturating_add(markdown.len()).saturating_add(2);
        page_records.push(PdfProgressivePage {
            page: page_index + 1,
            markdown,
            plain_text,
            char_start,
            char_end: cursor,
            figure_caption_count: signals.figure_caption_count,
            table_caption_count: signals.table_caption_count,
            url_count: signals.url_count,
        });
    }
    Ok(PdfProgressiveSourceBundle {
        input_profile: "md_light_plain_refs".to_string(),
        text_sha256: sha256_hex(text.as_bytes()),
        text,
        raw_markdown,
        raw_plain_text,
        reference_tail,
        pages: page_records,
        warnings: Vec::new(),
    })
}

fn reference_tail_from_plain_text(content: &str) -> String {
    let lower = content.to_ascii_lowercase();
    let Some(start) = lower
        .rfind("\nreferences")
        .or_else(|| lower.rfind("\nreference"))
    else {
        return String::new();
    };
    clean_text(&content[start..], TextProcessingMode::Light)
        .content
        .chars()
        .take(30_000)
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[derive(Debug, Default)]
struct PageLayoutSignalReport {
    warnings: Vec<String>,
}

fn collect_page_layout_signals(
    doc: &pdf_oxide::PdfDocument,
    pages: usize,
) -> PageLayoutSignalReport {
    let mut report = PageLayoutSignalReport::default();
    let mut blank_text_pages = 0usize;
    let mut image_like_text_pages = 0usize;
    let mut fragmented_pages = 0usize;
    let mut extraction_errors = Vec::new();

    for page_index in 0..pages {
        match (
            doc.extract_page_text(page_index),
            doc.extract_text_lines(page_index),
        ) {
            (Ok(page_text), Ok(lines)) => {
                let spans = page_text.spans.len();
                let chars = page_text.chars.len();
                let line_count = lines.len();
                if chars == 0 {
                    blank_text_pages += 1;
                    if let Ok(images) = doc.extract_images(page_index) {
                        if !images.is_empty() {
                            image_like_text_pages += 1;
                        }
                    }
                }
                if chars > 0 && spans > chars.saturating_div(2).max(24) {
                    fragmented_pages += 1;
                    report.warnings.push(format!(
                        "page {} has fragmented text layout: spans={spans}, chars={chars}, lines={line_count}",
                        page_index + 1
                    ));
                }
            }
            (Err(err), _) | (_, Err(err)) => {
                if extraction_errors.len() < 5 {
                    extraction_errors.push(format!("page {} layout signal: {err}", page_index + 1));
                }
            }
        }
    }

    if blank_text_pages > 0 {
        report.warnings.push(format!(
            "{blank_text_pages} pages have no extracted text chars"
        ));
    }
    if image_like_text_pages > 0 {
        report.warnings.push(format!(
            "{image_like_text_pages} pages look image-only; OCR would be required for text"
        ));
    }
    if fragmented_pages > 0 {
        report.warnings.push(format!(
            "{fragmented_pages} pages have highly fragmented text spans"
        ));
    }
    report.warnings.extend(extraction_errors);
    report
}

fn write_markdown_artifact(path: &str, content: &str) -> Result<AcademicParseArtifact> {
    let path = PathBuf::from(path);
    let bytes = write_text_file_no_overwrite(&path, content)?;
    Ok(AcademicParseArtifact {
        kind: "markdown".to_string(),
        status: "written".to_string(),
        path: Some(path.display().to_string()),
        count: None,
        bytes: Some(bytes),
        reason: None,
    })
}

fn write_raw_content_artifact(path: &str, content: &str) -> Result<AcademicParseArtifact> {
    let path = PathBuf::from(path);
    let bytes = write_text_file_no_overwrite(&path, content)?;
    Ok(AcademicParseArtifact {
        kind: "raw_content".to_string(),
        status: "written".to_string(),
        path: Some(path.display().to_string()),
        count: None,
        bytes: Some(bytes),
        reason: None,
    })
}

fn ensure_dir_artifact(kind: &str, dir: Option<&str>) -> Result<()> {
    if let Some(dir) = dir {
        ensure_output_dir(Path::new(dir), kind)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_artifact_writes_file_and_rejects_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("paper.md");
        let artifact = write_markdown_artifact(path.to_str().unwrap(), "# Paper")
            .expect("write markdown artifact");
        assert_eq!(artifact.kind, "markdown");
        assert_eq!(artifact.status, "written");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Paper");

        let err = write_markdown_artifact(path.to_str().unwrap(), "# Again")
            .expect_err("existing artifact should be rejected");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn image_table_dirs_are_created_without_extraction() {
        let dir = tempfile::tempdir().expect("tempdir");
        let images = dir.path().join("images");
        let tables = dir.path().join("tables");
        ensure_dir_artifact("images", images.to_str()).expect("ensure images dir");
        ensure_dir_artifact("tables", tables.to_str()).expect("ensure tables dir");
        assert!(images.is_dir());
        assert!(tables.is_dir());
    }

    #[test]
    fn extract_images_requires_output_dir() {
        let pdf = image_pdf_bytes();
        let err = match parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                extract_images: Some(true),
                ..Default::default()
            }),
        ) {
            Ok(_) => panic!("missing images_dir should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn missing_artifact_dir_fails_before_writing_markdown() {
        let pdf = image_pdf_bytes();
        let dir = tempfile::tempdir().expect("tempdir");
        let markdown = dir.path().join("paper.md");
        let err = match parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                save_markdown_path: Some(markdown.display().to_string()),
                extract_images: Some(true),
                ..Default::default()
            }),
        ) {
            Ok(_) => panic!("missing images_dir should fail"),
            Err(err) => err,
        };

        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
        assert!(!markdown.exists());
    }

    #[test]
    fn image_artifact_writes_png_and_manifest() {
        let pdf = image_pdf_bytes();
        let dir = tempfile::tempdir().expect("tempdir");
        let images = dir.path().join("images");
        let details = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                images_dir: Some(images.display().to_string()),
                extract_images: Some(true),
                ..Default::default()
            }),
        )
        .expect("parse image pdf");

        let artifact = details
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "images")
            .expect("images artifact");
        assert_eq!(artifact.status, "written");
        assert_eq!(artifact.count, Some(1));
        let manifest_path = images.join("images.json");
        assert_eq!(
            artifact.path.as_deref(),
            Some(manifest_path.to_str().unwrap())
        );
        assert!(manifest_path.is_file());
        assert!(images.join("page-0001-image-001.png").is_file());
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["written"], 1);
        assert_eq!(manifest["pipeline"][0], "collect_pdf_oxide_image_objects");
        assert_eq!(manifest["entries"][0]["status"], "written");
        assert_eq!(manifest["entries"][0]["rank"], 1);
    }

    #[test]
    fn tables_artifact_writes_empty_manifest_for_pdf_without_tables() {
        let pdf = image_pdf_bytes();
        let dir = tempfile::tempdir().expect("tempdir");
        let tables = dir.path().join("tables");
        let details = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                tables_dir: Some(tables.display().to_string()),
                extract_tables: Some(true),
                ..Default::default()
            }),
        )
        .expect("parse tables pdf");

        let artifact = details
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "tables")
            .expect("tables artifact");
        assert_eq!(artifact.status, "empty");
        assert_eq!(artifact.count, Some(0));
        assert!(tables.join("tables.json").is_file());
    }

    #[test]
    fn pipeline_defaults_to_clean_and_reports_passes() {
        let pdf = image_pdf_bytes();
        let details =
            parse_pdf_bytes_detailed(&pdf, "markdown", Some(10), None).expect("parse image pdf");

        assert_eq!(details.processing.text_processing_mode, "clean");
        assert!(details.raw_content.is_none());
        assert!(details.raw_original_length.is_none());
        assert_eq!(
            details.processing.processed_original_length,
            details.original_length
        );
        assert!(details
            .processing
            .passes
            .iter()
            .any(|pass| pass.name == "raw_text_extraction"));
        assert!(details
            .processing
            .passes
            .iter()
            .any(|pass| pass.name == "text_clean"));
        assert!(details.progressive_source.is_none());
        assert!(!details.pdf_sha256.is_empty());
    }

    #[test]
    fn llm_progressive_option_builds_source_bundle() {
        let pdf = image_pdf_bytes();
        let details = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                llm_progressive: Some(grok_search_types::AcademicLlmProgressiveOptions {
                    enabled: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .expect("parse image pdf");

        let source = details.progressive_source.expect("progressive source");
        assert_eq!(source.input_profile, "md_light_plain_refs");
        assert_eq!(source.pages.len(), 1);
        assert!(!source.text_sha256.is_empty());
    }

    #[test]
    fn include_raw_content_returns_raw_metadata() {
        let pdf = image_pdf_bytes();
        let details = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            Some(1),
            Some(&AcademicParseOptions {
                include_raw_content: Some(true),
                ..Default::default()
            }),
        )
        .expect("parse image pdf");

        assert!(details.raw_content.is_some());
        assert_eq!(
            details.raw_original_length,
            Some(details.processing.raw_original_length)
        );
        assert_eq!(
            details.raw_truncated,
            Some(details.processing.raw_truncated)
        );
    }

    #[test]
    fn raw_content_artifact_writes_file_without_inlining() {
        let pdf = image_pdf_bytes();
        let dir = tempfile::tempdir().expect("tempdir");
        let raw_path = dir.path().join("raw.md");
        let details = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            Some(1),
            Some(&AcademicParseOptions {
                save_raw_content_path: Some(raw_path.display().to_string()),
                ..Default::default()
            }),
        )
        .expect("parse image pdf");

        assert!(raw_path.is_file());
        assert!(details.raw_content.is_none());
        assert!(details.raw_original_length.is_none());
        assert!(details.processing.raw_truncated);
        assert!(details
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "raw_content"));
    }

    #[test]
    fn invalid_text_processing_mode_is_rejected() {
        let pdf = image_pdf_bytes();
        let err = parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                text_processing_mode: Some("heavy".to_string()),
                ..Default::default()
            }),
        )
        .expect_err("invalid mode should fail");

        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn existing_manifest_is_rejected() {
        let pdf = image_pdf_bytes();
        let dir = tempfile::tempdir().expect("tempdir");
        let images = dir.path().join("images");
        std::fs::create_dir_all(&images).expect("create images dir");
        std::fs::write(images.join("images.json"), "{}").expect("seed manifest");

        let err = match parse_pdf_bytes_detailed(
            &pdf,
            "markdown",
            None,
            Some(&AcademicParseOptions {
                images_dir: Some(images.display().to_string()),
                extract_images: Some(true),
                ..Default::default()
            }),
        ) {
            Ok(_) => panic!("existing manifest should fail"),
            Err(err) => err,
        };
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn tiny_image_filter_reason_is_stable() {
        assert_eq!(
            artifacts::image_filter_reason(20, 20, None).as_deref(),
            Some("tiny_pixel")
        );
    }

    fn image_pdf_bytes() -> Vec<u8> {
        let mut doc = pdf_oxide::writer::DocumentBuilder::new();
        doc.page(pdf_oxide::writer::PageSize::Letter)
            .image_from_bytes(
                &solid_png_bytes(300, 200),
                pdf_oxide::geometry::Rect::new(72.0, 500.0, 300.0, 200.0),
            )
            .expect("image")
            .done();
        doc.build().expect("build pdf")
    }

    fn solid_png_bytes(width: u32, height: u32) -> Vec<u8> {
        use image::ImageEncoder;

        let pixels = vec![128u8; (width * height * 3) as usize];
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgb8)
            .expect("encode png");
        bytes
    }
}
