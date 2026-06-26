use std::io::Write;

use grok_search_net::http::get_bytes;
use grok_search_types::{GrokSearchError, Result};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, USER_AGENT};

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";

pub struct ParsedContent {
    pub content: String,
    pub original_length: usize,
    pub truncated: bool,
}

pub async fn download_pdf_bytes(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let first = download_pdf_bytes_with_accept(client, url, "application/pdf").await;
    let bytes = match first {
        Ok(bytes) => bytes,
        Err(first_err) => match download_pdf_bytes_with_accept(client, url, "*/*").await {
            Ok(bytes) => bytes,
            Err(second_err) => {
                return Err(GrokSearchError::Provider(format!(
                    "academic pdf download failed for {url}: first attempt: {first_err}; retry with broad Accept: {second_err}"
                )))
            }
        },
    };
    validate_pdf_bytes(&bytes, max_bytes).map_err(|err| {
        GrokSearchError::Provider(format!("academic pdf validation failed for {url}: {err}"))
    })?;
    Ok(bytes)
}

async fn download_pdf_bytes_with_accept(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
) -> Result<Vec<u8>> {
    get_bytes(
        client,
        url,
        &[
            (USER_AGENT, UA),
            (ACCEPT, accept),
            (ACCEPT_ENCODING, "identity"),
        ],
        "academic pdf",
    )
    .await
}

pub fn validate_pdf_bytes(bytes: &[u8], max_bytes: usize) -> Result<()> {
    if bytes.len() > max_bytes {
        return Err(GrokSearchError::Provider(format!(
            "academic pdf exceeds max size: {} > {}",
            bytes.len(),
            max_bytes
        )));
    }
    if !bytes.starts_with(b"%PDF") {
        return Err(GrokSearchError::Provider(
            "resolved academic full text is not a PDF".to_string(),
        ));
    }
    Ok(())
}

pub fn parse_pdf_bytes(
    bytes: &[u8],
    format: &str,
    max_chars: Option<usize>,
) -> Result<ParsedContent> {
    let mut file = tempfile::NamedTempFile::new()
        .map_err(|err| GrokSearchError::Provider(format!("create temp PDF: {err}")))?;
    file.write_all(bytes)
        .map_err(|err| GrokSearchError::Provider(format!("write temp PDF: {err}")))?;
    let path = file.path().to_path_buf();
    let content = parse_with_pdf_oxide(&path, format)?;
    Ok(truncate_content(content, max_chars))
}

pub fn truncate_content(content: String, max_chars: Option<usize>) -> ParsedContent {
    let original_length = content.chars().count();
    let mut truncated = false;
    let content = if let Some(limit) = max_chars {
        if original_length > limit {
            truncated = true;
            content.chars().take(limit).collect()
        } else {
            content
        }
    } else {
        content
    };
    ParsedContent {
        content,
        original_length,
        truncated,
    }
}

fn parse_with_pdf_oxide(path: &std::path::Path, format: &str) -> Result<String> {
    let doc = pdf_oxide::PdfDocument::open(path)
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide open: {err}")))?;
    let pages = doc
        .page_count()
        .map_err(|err| GrokSearchError::Parse(format!("pdf_oxide page_count: {err}")))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pdf_bytes() {
        assert!(validate_pdf_bytes(b"not-pdf", 100).is_err());
    }

    #[test]
    fn rejects_oversized_pdf() {
        assert!(validate_pdf_bytes(b"%PDF-1.7", 3).is_err());
    }

    #[test]
    fn truncates_content_by_chars() {
        let parsed = truncate_content("abcdef".to_string(), Some(3));
        assert_eq!(parsed.content, "abc");
        assert_eq!(parsed.original_length, 6);
        assert!(parsed.truncated);
    }
}
