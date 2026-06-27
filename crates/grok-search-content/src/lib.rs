use std::io::Write;

use grok_search_net::http::get_bytes;
use grok_search_types::{GrokSearchError, Result};
use reqwest::header::{HeaderName, ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, USER_AGENT};

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

pub struct PdfDownloadOptions<'a> {
    pub label: &'a str,
    pub warmup_url: Option<&'a str>,
    pub headers: &'a [(HeaderName, &'a str)],
}

pub async fn download_pdf_bytes_with_options(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    options: PdfDownloadOptions<'_>,
) -> Result<Vec<u8>> {
    if let Some(warmup_url) = options.warmup_url {
        let mut builder = client.get(warmup_url);
        for (name, value) in options.headers {
            builder = builder.header(name.clone(), *value);
        }
        let _ = builder.send().await.map_err(|err| {
            if err.is_timeout() {
                GrokSearchError::Timeout(format!("{} warmup timed out: {err}", options.label))
            } else {
                GrokSearchError::Provider(format!("{} warmup failed: {err}", options.label))
            }
        })?;
    }
    let response = request_with_headers(client, url, options.headers, options.label).await?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| {
            GrokSearchError::Provider(format!("{} body read failed: {err}", options.label))
        })?
        .to_vec();
    if !status.is_success() {
        return Err(GrokSearchError::Provider(format!(
            "{} returned HTTP {status}: {}",
            options.label,
            String::from_utf8_lossy(&bytes)
        )));
    }
    if !content_type.contains("application/pdf") && !bytes.starts_with(b"%PDF") {
        return Err(GrokSearchError::Provider(format!(
            "{} resolved content is not a PDF (content-type: {})",
            options.label,
            if content_type.is_empty() {
                "unknown"
            } else {
                content_type.as_str()
            }
        )));
    }
    validate_pdf_bytes(&bytes, max_bytes).map_err(|err| {
        GrokSearchError::Provider(format!(
            "{} validation failed for {url}: {err}",
            options.label
        ))
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

async fn request_with_headers(
    client: &reqwest::Client,
    url: &str,
    headers: &[(HeaderName, &str)],
    label: &str,
) -> Result<reqwest::Response> {
    let mut builder = client.get(url);
    for (name, value) in headers {
        builder = builder.header(name.clone(), *value);
    }
    builder.send().await.map_err(|err| {
        if err.is_timeout() {
            GrokSearchError::Timeout(format!("{label} GET timed out: {err}"))
        } else {
            GrokSearchError::Provider(format!("{label} GET failed: {err}"))
        }
    })
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

    #[tokio::test]
    async fn option_download_rejects_html_even_with_success_status() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/paper", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let body = b"<html>challenge</html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });

        let err = download_pdf_bytes_with_options(
            &reqwest::Client::new(),
            &url,
            1024,
            PdfDownloadOptions {
                label: "test pdf",
                warmup_url: None,
                headers: &[],
            },
        )
        .await
        .expect_err("html should be rejected");
        assert!(err.to_string().contains("not a PDF"), "{err}");
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
