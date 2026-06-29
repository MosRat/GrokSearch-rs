use std::io::Write;
use std::path::{Path, PathBuf};

use grok_search_content::{
    ensure_output_dir, truncate_content, write_text_file_no_overwrite, ParsedContent,
};
use grok_search_types::{
    AcademicParseArtifact, AcademicParseCapabilities, AcademicParseOptions, GrokSearchError, Result,
};

use crate::artifacts;

pub struct ParsedPdfDetails {
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
    pub artifacts: Vec<AcademicParseArtifact>,
    pub capabilities: AcademicParseCapabilities,
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
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|err| GrokSearchError::Io(format!("create temp PDF: {err}")))?;
    file.write_all(bytes)
        .map_err(|err| GrokSearchError::Io(format!("write temp PDF: {err}")))?;
    let path = file.path().to_path_buf();
    let doc = open_pdf_oxide_document(&path)?;
    let pages = doc
        .page_count()
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide page_count: {err}")))?;
    let content = extract_text_with_pdf_oxide(&doc, pages, format)?;
    let parsed = truncate_content(content, max_chars);
    let mut artifacts = Vec::new();
    let capabilities = AcademicParseCapabilities::default();
    if let Some(options) = options {
        validate_parse_artifact_options(options)?;
        if let Some(path) = options.save_markdown_path.as_deref() {
            artifacts.push(write_markdown_artifact(path, &parsed.content)?);
        }
        if options.extract_images.unwrap_or(false) {
            artifacts.push(artifacts::extract_image_artifacts(
                &doc,
                pages,
                options.images_dir.as_deref(),
            )?);
        } else if options.images_dir.is_some() {
            ensure_dir_artifact("images", options.images_dir.as_deref())?;
        }
        if options.extract_tables.unwrap_or(false) {
            artifacts.push(artifacts::extract_table_artifacts(
                &doc,
                pages,
                options.tables_dir.as_deref(),
            )?);
        } else if options.tables_dir.is_some() {
            ensure_dir_artifact("tables", options.tables_dir.as_deref())?;
        }
    }
    Ok(ParsedPdfDetails {
        content: parsed.content,
        original_length: parsed.original_length,
        truncated: parsed.truncated,
        artifacts,
        capabilities,
    })
}

fn validate_parse_artifact_options(options: &AcademicParseOptions) -> Result<()> {
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

fn open_pdf_oxide_document(path: &std::path::Path) -> Result<pdf_oxide::PdfDocument> {
    pdf_oxide::PdfDocument::open(path)
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide open: {err}")))
}

fn extract_text_with_pdf_oxide(
    doc: &pdf_oxide::PdfDocument,
    pages: usize,
    format: &str,
) -> Result<String> {
    let mut out = String::new();
    for page in 0..pages {
        let text = if format == "markdown" {
            doc.to_markdown(page, &pdf_oxide::converters::ConversionOptions::default())
        } else {
            doc.extract_text(page)
        }
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide extract page {page}: {err}")))?;
        out.push_str(&text);
        out.push_str("\n\n");
    }
    Ok(out)
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
        assert_eq!(artifact.path.as_deref(), Some(manifest_path.to_str().unwrap()));
        assert!(manifest_path.is_file());
        assert!(images.join("page-0001-image-001.png").is_file());
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["written"], 1);
        assert_eq!(manifest["entries"][0]["status"], "written");
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
