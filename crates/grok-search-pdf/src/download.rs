use std::sync::Arc;

use grok_search_net::http::{build_client_direct, get_bytes_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_types::{GrokSearchError, Result};
use reqwest::header::{
    HeaderMap, HeaderName, ACCEPT, ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    RANGE, USER_AGENT,
};
use tokio::sync::Semaphore;

use crate::validate_pdf_bytes;

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";
const RANGE_FIRST_HOSTS: &[&str] = &[
    "arxiv.org",
    "export.arxiv.org",
    "openaccess.thecvf.com",
    "aclanthology.org",
];

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

#[derive(Debug, Clone)]
pub struct OptimizedPdfDownloadOptions {
    pub timeout: std::time::Duration,
    pub max_bytes: usize,
    pub max_response_bytes: usize,
    pub range_chunk_size: usize,
    pub range_concurrency: usize,
    pub enable_direct_fallback: bool,
}

impl OptimizedPdfDownloadOptions {
    pub fn new(timeout: std::time::Duration, max_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            timeout,
            max_bytes,
            max_response_bytes,
            range_chunk_size: 256 * 1024,
            range_concurrency: 4,
            enable_direct_fallback: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizedPdfDownloadOutcome {
    pub bytes: Vec<u8>,
    pub plan: String,
    pub strategy: String,
    pub attempts: Vec<PdfDownloadAttemptReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfDownloadAttemptReport {
    pub strategy: String,
    pub elapsed_ms: u64,
    pub bytes: u64,
    pub status: String,
    pub error: Option<String>,
}

pub async fn download_pdf_bytes_optimized(
    client: &reqwest::Client,
    url: &str,
    options: OptimizedPdfDownloadOptions,
) -> Result<OptimizedPdfDownloadOutcome> {
    let mut attempts = Vec::new();
    let mut last_error = None;
    let (plan, candidates) = optimized_candidates(url, &options)?;
    for candidate in candidates {
        let started = std::time::Instant::now();
        let result = match candidate.client {
            DownloadClient::Current => match candidate.kind {
                DownloadStrategyKind::Full => {
                    download_pdf_bytes_limited(
                        client,
                        url,
                        options.max_bytes,
                        options.max_response_bytes,
                    )
                    .await
                }
                DownloadStrategyKind::Range => {
                    segmented_range_download(client, url, &options).await
                }
            },
            DownloadClient::Direct(direct_client) => match candidate.kind {
                DownloadStrategyKind::Full => {
                    download_pdf_bytes_limited(
                        &direct_client,
                        url,
                        options.max_bytes,
                        options.max_response_bytes,
                    )
                    .await
                }
                DownloadStrategyKind::Range => {
                    segmented_range_download(&direct_client, url, &options).await
                }
            },
        };
        match result {
            Ok(bytes) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let strategy = candidate.name.to_string();
                attempts.push(PdfDownloadAttemptReport {
                    strategy: strategy.clone(),
                    elapsed_ms,
                    bytes: bytes.len() as u64,
                    status: "ok".to_string(),
                    error: None,
                });
                return Ok(OptimizedPdfDownloadOutcome {
                    bytes,
                    plan,
                    strategy,
                    attempts,
                });
            }
            Err(err) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                attempts.push(PdfDownloadAttemptReport {
                    strategy: candidate.name.to_string(),
                    elapsed_ms,
                    bytes: 0,
                    status: "failed".to_string(),
                    error: Some(err.to_string()),
                });
                last_error = Some(err);
            }
        }
    }
    let detail = attempts
        .iter()
        .map(|attempt| {
            format!(
                "{}={}ms {}",
                attempt.strategy,
                attempt.elapsed_ms,
                attempt.error.as_deref().unwrap_or("failed")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(last_error.unwrap_or_else(|| {
        GrokSearchError::Upstream(format!("optimized PDF download failed for {url}: {detail}"))
    }))
}

#[derive(Clone)]
struct DownloadCandidate {
    name: &'static str,
    kind: DownloadStrategyKind,
    client: DownloadClient,
}

#[derive(Clone, Copy)]
enum DownloadStrategyKind {
    Full,
    Range,
}

#[derive(Clone)]
enum DownloadClient {
    Current,
    Direct(reqwest::Client),
}

fn optimized_candidates(
    url: &str,
    options: &OptimizedPdfDownloadOptions,
) -> Result<(String, Vec<DownloadCandidate>)> {
    let host = pdf_download_host(url).unwrap_or_else(|| "unknown".to_string());
    let range_first = prefers_range_first_host(&host);
    let plan = format!(
        "adaptive host={host} range_first={range_first} direct_fallback={} range_chunk_size={} range_concurrency={}",
        options.enable_direct_fallback,
        options.range_chunk_size,
        options.range_concurrency.clamp(1, 8)
    );
    let mut candidates = Vec::new();
    if range_first {
        candidates.push(current_candidate(
            "current_range_parallel",
            DownloadStrategyKind::Range,
        ));
        candidates.push(current_candidate(
            "current_full",
            DownloadStrategyKind::Full,
        ));
    } else {
        candidates.push(current_candidate(
            "current_full",
            DownloadStrategyKind::Full,
        ));
        candidates.push(current_candidate(
            "current_range_parallel",
            DownloadStrategyKind::Range,
        ));
    }
    if options.enable_direct_fallback {
        let direct = build_client_direct(options.timeout)?;
        candidates.push(direct_candidate(
            "direct_range_parallel",
            DownloadStrategyKind::Range,
            direct.clone(),
        ));
        candidates.push(direct_candidate(
            "direct_full",
            DownloadStrategyKind::Full,
            direct,
        ));
    }
    Ok((plan, candidates))
}

fn current_candidate(name: &'static str, kind: DownloadStrategyKind) -> DownloadCandidate {
    DownloadCandidate {
        name,
        kind,
        client: DownloadClient::Current,
    }
}

fn direct_candidate(
    name: &'static str,
    kind: DownloadStrategyKind,
    client: reqwest::Client,
) -> DownloadCandidate {
    DownloadCandidate {
        name,
        kind,
        client: DownloadClient::Direct(client),
    }
}

fn pdf_download_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
}

fn prefers_range_first_host(host: &str) -> bool {
    RANGE_FIRST_HOSTS
        .iter()
        .any(|candidate| host == *candidate || host.ends_with(&format!(".{candidate}")))
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

async fn segmented_range_download(
    client: &reqwest::Client,
    url: &str,
    options: &OptimizedPdfDownloadOptions,
) -> Result<Vec<u8>> {
    let meta = probe_pdf_head(client, url).await?;
    let Some(total_len) = meta.content_length else {
        return Err(GrokSearchError::Upstream(
            "range PDF download requires content-length".to_string(),
        ));
    };
    if total_len == 0 || total_len > options.max_bytes as u64 {
        return Err(GrokSearchError::Provider(format!(
            "academic pdf size invalid for range download: {total_len} > {}",
            options.max_bytes
        )));
    }
    if !meta.accept_ranges && total_len > options.range_chunk_size as u64 {
        return Err(GrokSearchError::Upstream(
            "range PDF download not supported by upstream".to_string(),
        ));
    }
    let chunk_size = options.range_chunk_size.max(64 * 1024) as u64;
    let concurrency = options.range_concurrency.clamp(1, 8);
    let ranges = build_ranges(total_len, chunk_size);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();
    for (start, end) in ranges {
        let permit = semaphore.clone().acquire_owned().await.map_err(|err| {
            GrokSearchError::Upstream(format!("range PDF semaphore closed: {err}"))
        })?;
        let client = client.clone();
        let url = url.to_string();
        handles.push(tokio::spawn(async move {
            let _permit = permit;
            let bytes = fetch_pdf_range(&client, &url, start, end).await?;
            Ok::<_, GrokSearchError>((start, bytes))
        }));
    }
    let mut chunks = Vec::new();
    for handle in handles {
        let item = handle
            .await
            .map_err(|err| GrokSearchError::Upstream(format!("range PDF task failed: {err}")))??;
        chunks.push(item);
    }
    chunks.sort_by_key(|(start, _)| *start);
    let mut out = Vec::with_capacity(total_len as usize);
    for (_, bytes) in chunks {
        if out.len().saturating_add(bytes.len()) > options.max_response_bytes.min(options.max_bytes)
        {
            return Err(GrokSearchError::Upstream(format!(
                "academic pdf response exceeded max_response_bytes={}",
                options.max_response_bytes.min(options.max_bytes)
            )));
        }
        out.extend_from_slice(&bytes);
    }
    if out.len() as u64 != total_len {
        return Err(GrokSearchError::Upstream(format!(
            "range PDF download incomplete: got {}, expected {total_len}",
            out.len()
        )));
    }
    validate_pdf_bytes(&out, options.max_bytes).map_err(|err| {
        GrokSearchError::Provider(format!("academic pdf validation failed for {url}: {err}"))
    })?;
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
struct PdfHeadMetadata {
    content_length: Option<u64>,
    accept_ranges: bool,
}

async fn probe_pdf_head(client: &reqwest::Client, url: &str) -> Result<PdfHeadMetadata> {
    let response = client
        .head(url)
        .header(USER_AGENT, UA)
        .header(ACCEPT, "application/pdf")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                GrokSearchError::Timeout(format!("academic pdf HEAD timed out: {err}"))
            } else {
                GrokSearchError::Upstream(format!("academic pdf HEAD failed: {err}"))
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(GrokSearchError::Upstream(format!(
            "academic pdf HEAD returned HTTP {status}"
        )));
    }
    let headers = response.headers();
    Ok(PdfHeadMetadata {
        content_length: parse_content_length(headers),
        accept_ranges: headers
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("bytes")),
    })
}

fn parse_content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn build_ranges(total_len: u64, chunk_size: u64) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < total_len {
        let end = start
            .saturating_add(chunk_size)
            .saturating_sub(1)
            .min(total_len - 1);
        ranges.push((start, end));
        start = end.saturating_add(1);
    }
    ranges
}

async fn fetch_pdf_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>> {
    let response = client
        .get(url)
        .header(USER_AGENT, UA)
        .header(ACCEPT, "application/pdf")
        .header(ACCEPT_ENCODING, "identity")
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                GrokSearchError::Timeout(format!(
                    "academic pdf range {start}-{end} timed out: {err}"
                ))
            } else {
                GrokSearchError::Upstream(format!("academic pdf range {start}-{end} failed: {err}"))
            }
        })?;
    let status = response.status();
    if status != reqwest::StatusCode::PARTIAL_CONTENT && status != reqwest::StatusCode::OK {
        return Err(GrokSearchError::Upstream(format!(
            "academic pdf range {start}-{end} returned HTTP {status}"
        )));
    }
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        verify_content_range(response.headers(), start, end)?;
    }
    let bytes =
        read_response_bytes(response, "academic pdf range", (end - start + 1) as usize).await?;
    let expected = (end - start + 1) as usize;
    if bytes.len() != expected && status == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(GrokSearchError::Upstream(format!(
            "academic pdf range {start}-{end} returned {} bytes, expected {expected}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn verify_content_range(headers: &HeaderMap, start: u64, end: u64) -> Result<()> {
    let Some(raw) = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    let expected = format!("bytes {start}-{end}/");
    if raw.starts_with(&expected) {
        return Ok(());
    }
    Err(GrokSearchError::Upstream(format!(
        "unexpected content-range for academic pdf range {start}-{end}: {raw}"
    )))
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

    #[test]
    fn adaptive_candidates_use_range_first_for_known_large_pdf_hosts() {
        let mut options =
            OptimizedPdfDownloadOptions::new(std::time::Duration::from_secs(1), 1024, 1024);
        options.enable_direct_fallback = false;
        let (plan, candidates) =
            optimized_candidates("https://arxiv.org/pdf/1706.03762", &options).expect("candidates");
        let names: Vec<_> = candidates.iter().map(|candidate| candidate.name).collect();

        assert!(plan.contains("adaptive host=arxiv.org range_first=true"));
        assert_eq!(names, vec!["current_range_parallel", "current_full"]);
    }

    #[test]
    fn adaptive_candidates_keep_full_first_for_default_hosts() {
        let mut options =
            OptimizedPdfDownloadOptions::new(std::time::Duration::from_secs(1), 1024, 1024);
        options.enable_direct_fallback = false;
        let (plan, candidates) =
            optimized_candidates("https://example.com/paper.pdf", &options).expect("candidates");
        let names: Vec<_> = candidates.iter().map(|candidate| candidate.name).collect();

        assert!(plan.contains("adaptive host=example.com range_first=false"));
        assert_eq!(names, vec!["current_full", "current_range_parallel"]);
    }

    #[tokio::test]
    async fn optimized_download_falls_back_to_range_segments() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::thread;

        let body = b"%PDF-1.7\nrange-download-test".to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_thread = Arc::clone(&requests);
        let server_body = body.clone();
        thread::spawn(move || {
            for _ in 0..10 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                let Ok(n) = stream.read(&mut buf) else {
                    continue;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                if req.starts_with("HEAD ") {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                        server_body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                } else if let Some((start, end)) = parse_test_range(&req) {
                    let end = end.min(server_body.len() - 1);
                    let slice = &server_body[start..=end];
                    let response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        server_body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(slice);
                } else {
                    let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfail";
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });

        let mut options =
            OptimizedPdfDownloadOptions::new(std::time::Duration::from_secs(5), 1024, 1024);
        options.range_chunk_size = 8;
        options.range_concurrency = 2;
        options.enable_direct_fallback = false;
        let outcome = download_pdf_bytes_optimized(
            &reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("client"),
            &url,
            options,
        )
        .await
        .expect("range fallback");

        assert_eq!(outcome.bytes, body);
        assert!(outcome.plan.contains("adaptive"));
        assert_eq!(outcome.strategy, "current_range_parallel");
        assert_eq!(outcome.attempts[0].strategy, "current_full");
        assert_eq!(outcome.attempts[0].status, "failed");
        assert_eq!(outcome.attempts[1].strategy, "current_range_parallel");
        assert!(requests.load(Ordering::SeqCst) >= 3);
    }

    fn parse_test_range(req: &str) -> Option<(usize, usize)> {
        let marker = "range: bytes=";
        let lower = req.to_ascii_lowercase();
        let start = lower.find(marker)? + marker.len();
        let tail = &req[start..];
        let end = tail.find("\r\n")?;
        let value = &tail[..end];
        let (start, end) = value.split_once('-')?;
        Some((start.parse().ok()?, end.parse().ok()?))
    }
}
