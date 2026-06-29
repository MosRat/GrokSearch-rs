use grok_search_net::http::{get_bytes_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_types::{GrokSearchError, Result};
use reqwest::header::{HeaderName, ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, USER_AGENT};

use crate::validate_pdf_bytes;

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";

pub async fn download_pdf_bytes(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    download_pdf_bytes_limited(client, url, max_bytes, DEFAULT_MAX_RESPONSE_BYTES).await
}

pub async fn download_pdf_bytes_limited(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    let read_limit = max_response_bytes.min(max_bytes);
    let first = download_pdf_bytes_with_accept(client, url, "application/pdf", read_limit).await;
    let bytes = match first {
        Ok(bytes) => bytes,
        Err(first_err) => match download_pdf_bytes_with_accept(client, url, "*/*", read_limit).await {
            Ok(bytes) => bytes,
            Err(second_err) => {
                return Err(GrokSearchError::Upstream(format!(
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
    download_pdf_bytes_with_options_limited(
        client,
        url,
        max_bytes,
        options,
        DEFAULT_MAX_RESPONSE_BYTES,
    )
    .await
}

pub async fn download_pdf_bytes_with_options_limited(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    options: PdfDownloadOptions<'_>,
    max_response_bytes: usize,
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
                GrokSearchError::Upstream(format!("{} warmup failed: {err}", options.label))
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
    let bytes =
        read_response_bytes(response, options.label, max_response_bytes.min(max_bytes)).await?;
    if !status.is_success() {
        return Err(GrokSearchError::Upstream(format!(
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
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    get_bytes_limited(
        client,
        url,
        &[
            (USER_AGENT, UA),
            (ACCEPT, accept),
            (ACCEPT_ENCODING, "identity"),
        ],
        "academic pdf",
        max_response_bytes,
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
            GrokSearchError::Upstream(format!("{label} GET failed: {err}"))
        }
    })
}

async fn read_response_bytes(
    mut response: reqwest::Response,
    label: &str,
    max_response_bytes: usize,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| GrokSearchError::Upstream(format!("{label} body read failed: {err}")))?
    {
        if out.len().saturating_add(chunk.len()) > max_response_bytes {
            return Err(GrokSearchError::Upstream(format!(
                "{label} response exceeded max_response_bytes={max_response_bytes}"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

