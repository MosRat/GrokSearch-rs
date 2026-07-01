use grok_search_pdf::{parse_pdf_bytes_detailed, ParsedPdfDetails};
use grok_search_provider_core::FullTextLocation;
use grok_search_types::{AcademicParseOptions, AcademicPdfLocator, GrokSearchError, Result};

pub(super) fn ensure_valid_locator(locator: &AcademicPdfLocator, tool_name: &str) -> Result<()> {
    if locator.is_valid_exactly_one() {
        return Ok(());
    }
    Err(GrokSearchError::InvalidParams(format!(
        "{tool_name} requires exactly one of identifier, url, or pdf_url"
    )))
}

pub(super) fn pdf_cache_key(location: &FullTextLocation, max_bytes: usize) -> String {
    let payload = format!(
        "academic_pdf:v1\nsource={}\nurl={}\nmax_bytes={max_bytes}",
        location.source, location.url
    );
    format!("academic_pdf:v1:{}", sha256_hex(payload.as_bytes()))
}

pub(super) fn pdf_url_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn pdf_download_retry_delay_ms(attempt: u32) -> u64 {
    match attempt {
        0 | 1 => 600,
        2 => 1_200,
        _ => 2_400,
    }
}

pub(super) fn is_retryable_pdf_download_error(err: &GrokSearchError) -> bool {
    match err {
        GrokSearchError::Timeout(_) => true,
        GrokSearchError::Upstream(message) | GrokSearchError::Provider(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("http 429")
                || lower.contains("too many requests")
                || lower.contains("rate limit")
                || lower.contains("http 500")
                || lower.contains("http 502")
                || lower.contains("http 503")
                || lower.contains("http 504")
                || lower.contains("timed out")
                || lower.contains("timeout")
                || lower.contains("connect")
                || lower.contains("connection")
                || lower.contains("body read failed")
                || lower.contains("request failed")
        }
        _ => false,
    }
}

pub(super) fn prefer_institutional_locations(
    locations: Vec<FullTextLocation>,
) -> Vec<FullTextLocation> {
    let mut unique: Vec<FullTextLocation> = Vec::new();
    for location in locations {
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| existing.url == location.url)
        {
            if is_institutional_source(&location.source)
                && !is_institutional_source(&existing.source)
            {
                *existing = location;
            }
        } else {
            unique.push(location);
        }
    }
    unique
}

fn is_institutional_source(source: &str) -> bool {
    matches!(source, "ieee_institutional" | "acm_institutional")
}

pub(super) async fn parse_pdf_bytes_with_timeout(
    bytes: Vec<u8>,
    format: String,
    limit: usize,
    options: Option<&AcademicParseOptions>,
    timeout: std::time::Duration,
    url: &str,
) -> Result<ParsedPdfDetails> {
    let url = url.to_string();
    let options = options.cloned();
    if timeout.is_zero() {
        return Err(GrokSearchError::Timeout(format!(
            "academic_read PDF parse timed out for {url}"
        )));
    }
    tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            parse_pdf_bytes_detailed(&bytes, &format, Some(limit), options.as_ref())
        }),
    )
    .await
    .map_err(|_| GrokSearchError::Timeout(format!("academic_read PDF parse timed out for {url}")))?
    .map_err(|err| GrokSearchError::Io(format!("academic_read parse task failed: {err}")))?
}
