mod arxiv;
mod crossref;
mod dblp;
mod http;
mod openalex;
mod rate_limit;
mod scihub;
mod semantic;
mod unpaywall;

pub(crate) use arxiv::ArxivProvider;
#[cfg(test)]
pub(crate) use arxiv::{arxiv_pdf_url, arxiv_search_query, arxiv_sort_by, parse_arxiv_atom};
pub(crate) use crossref::CrossrefProvider;
#[cfg(test)]
pub(crate) use crossref::{crossref_filter, crossref_sort, parse_crossref_work};
#[cfg(test)]
pub(crate) use dblp::parse_dblp_search;
pub(crate) use dblp::DblpProvider;
#[cfg(test)]
pub(crate) use openalex::{openalex_filter, openalex_sort, parse_openalex_work};
pub(crate) use openalex::{without_openalex_reference_sources, OpenAlexProvider};
pub(crate) use scihub::SciHubProvider;
pub(crate) use semantic::SemanticProvider;
#[cfg(test)]
pub(crate) use semantic::{parse_semantic_paper, semantic_sort, semantic_year_filter};
pub(crate) use unpaywall::UnpaywallProvider;

fn sort_is(sort_by: Option<&str>, expected: &str) -> bool {
    sort_by
        .unwrap_or("relevance")
        .trim()
        .eq_ignore_ascii_case(expected)
}

fn clean_title(title: &str) -> String {
    grok_search_parse::clean_html_title(title)
}

#[cfg(test)]
mod tests {
    use super::rate_limit::{unix_millis, wait_for_global_provider_rate_limit};
    use super::*;
    use grok_search_net::http::DEFAULT_MAX_RESPONSE_BYTES;
    use grok_search_types::AcademicSearchInput;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Duration as StdDuration};
    use tokio::time::Instant;
    use url::Url;

    const ARXIV_OK_BODY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <id>http://arxiv.org/abs/2401.00001v1</id>
    <title>Test Paper</title>
    <summary>Test summary</summary>
    <published>2024-01-01T00:00:00Z</published>
    <author><name>Ada Lovelace</name></author>
    <link href="http://arxiv.org/pdf/2401.00001v1" type="application/pdf"/>
  </entry>
</feed>"#;

    const OPENALEX_OK_BODY: &str = r#"{"id":"https://openalex.org/W1","title":"Test Work","results":[{"id":"https://openalex.org/W1","title":"Test Work"}]}"#;

    struct MockResponse {
        status: u16,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    }

    fn spawn_mock_server(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_log = Arc::clone(&requests);
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buf = [0u8; 4096];
                let read = stream.read(&mut buf).expect("read request");
                request_log
                    .lock()
                    .expect("request log")
                    .push(String::from_utf8_lossy(&buf[..read]).into_owned());
                let reason = match response.status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    502 => "Bad Gateway",
                    503 => "Service Unavailable",
                    504 => "Gateway Timeout",
                    _ => "Status",
                };
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    reason,
                    response.body.len()
                )
                .expect("write status");
                for (name, value) in response.headers {
                    write!(stream, "{name}: {value}\r\n").expect("write header");
                }
                write!(stream, "\r\n{}", response.body).expect("write body");
                stream.flush().expect("flush response");
            }
        });
        (base, requests)
    }

    fn response(status: u16, body: &'static str) -> MockResponse {
        MockResponse {
            status,
            body,
            headers: Vec::new(),
        }
    }

    fn response_with_headers(
        status: u16,
        body: &'static str,
        headers: Vec<(&'static str, &'static str)>,
    ) -> MockResponse {
        MockResponse {
            status,
            body,
            headers,
        }
    }

    #[test]
    fn openalex_adds_rotated_api_key_query_parameter() {
        let provider = OpenAlexProvider::new(
            reqwest::Client::new(),
            Some("person@example.com".to_string()),
            Some("oa-a, oa-b".to_string()),
        );
        let mut first = Url::parse("https://api.openalex.org/works").unwrap();
        provider.add_mailto(&mut first);
        provider.add_key(&mut first, 0);
        assert_eq!(
            first.query(),
            Some("mailto=person%40example.com&api_key=oa-a")
        );

        let mut second = Url::parse("https://api.openalex.org/works").unwrap();
        provider.add_key(&mut second, 1);
        assert_eq!(second.query(), Some("api_key=oa-b"));
    }

    #[test]
    fn arxiv_pdf_url_normalizes_common_ids() {
        assert_eq!(
            arxiv_pdf_url("1706.03762"),
            "https://arxiv.org/pdf/1706.03762"
        );
        assert_eq!(
            arxiv_pdf_url("1706.03762v7"),
            "https://arxiv.org/pdf/1706.03762v7"
        );
        assert_eq!(
            arxiv_pdf_url("arXiv:1706.03762.pdf"),
            "https://arxiv.org/pdf/1706.03762"
        );
    }

    #[test]
    fn semantic_year_filter_supports_ranges_and_open_ends() {
        assert_eq!(
            semantic_year_filter(Some(2024), Some(2024)).as_deref(),
            Some("2024")
        );
        assert_eq!(
            semantic_year_filter(Some(2020), Some(2024)).as_deref(),
            Some("2020-2024")
        );
        assert_eq!(
            semantic_year_filter(Some(2020), None).as_deref(),
            Some("2020-")
        );
        assert_eq!(
            semantic_year_filter(None, Some(2024)).as_deref(),
            Some("-2024")
        );
        assert!(semantic_year_filter(None, None).is_none());
    }

    #[test]
    fn provider_sort_params_map_common_academic_preferences() {
        assert_eq!(semantic_sort(Some("citations")), Some("citationCount:desc"));
        assert_eq!(semantic_sort(Some("date")), Some("publicationDate:desc"));
        assert_eq!(semantic_sort(Some("relevance")), None);
        assert_eq!(arxiv_sort_by(Some("date")), "submittedDate");
        assert_eq!(arxiv_sort_by(Some("citations")), "relevance");
        let openalex_citations = AcademicSearchInput {
            sort_by: Some("citations".to_string()),
            ..Default::default()
        };
        assert_eq!(
            openalex_sort(&openalex_citations),
            Some("cited_by_count:desc")
        );
        let openalex_broad_date = AcademicSearchInput {
            sort_by: Some("date".to_string()),
            ..Default::default()
        };
        assert_eq!(openalex_sort(&openalex_broad_date), None);
        let openalex_filtered_date = AcademicSearchInput {
            sort_by: Some("date".to_string()),
            year_from: Some(2024),
            ..Default::default()
        };
        assert_eq!(
            openalex_sort(&openalex_filtered_date),
            Some("publication_date:desc")
        );
        assert_eq!(
            crossref_sort(Some("citations")),
            Some(("is-referenced-by-count", "desc"))
        );
        assert_eq!(crossref_sort(Some("date")), Some(("published", "desc")));
    }

    #[test]
    fn arxiv_search_query_rewrites_plain_text_to_all_terms() {
        assert_eq!(
            arxiv_search_query("large language model evaluation"),
            "all:large AND all:language AND all:model AND all:evaluation"
        );
        assert_eq!(
            arxiv_search_query("Attention Is All You Need"),
            "all:attention AND all:need"
        );
        assert_eq!(arxiv_search_query("ti:transformer"), "ti:transformer");
    }

    #[test]
    fn openalex_filter_includes_dates_and_open_access() {
        let mut input = AcademicSearchInput {
            year_from: Some(2024),
            year_to: Some(2025),
            open_access_only: Some(true),
            ..Default::default()
        };
        assert_eq!(
            openalex_filter(&input).as_deref(),
            Some("from_publication_date:2024-01-01,to_publication_date:2025-12-31,is_oa:true")
        );
        input.year_from = None;
        assert_eq!(
            openalex_filter(&input).as_deref(),
            Some("to_publication_date:2025-12-31,is_oa:true")
        );
        assert!(openalex_filter(&AcademicSearchInput::default()).is_none());
    }

    #[test]
    fn crossref_filter_uses_publication_date_bounds() {
        assert_eq!(
            crossref_filter(Some(2024), Some(2025)).as_deref(),
            Some("from-pub-date:2024-01-01,until-pub-date:2025-12-31")
        );
        assert_eq!(
            crossref_filter(None, Some(2025)).as_deref(),
            Some("until-pub-date:2025-12-31")
        );
        assert!(crossref_filter(None, None).is_none());
    }

    #[tokio::test]
    async fn arxiv_429_retries_and_returns_xml() {
        let (base, requests) = spawn_mock_server(vec![
            response_with_headers(429, "rate limited", vec![("Retry-After", "0")]),
            response(200, ARXIV_OK_BODY),
        ]);
        let provider = ArxivProvider::new(reqwest::Client::builder().build().unwrap());
        let papers = provider
            .get_text_with_rate_limit_interval(
                &format!("{base}/api/query?id_list=2401.00001"),
                Duration::ZERO,
            )
            .await
            .and_then(|xml| parse_arxiv_atom(&xml))
            .expect("arxiv retry should succeed");

        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].arxiv_id.as_deref(), Some("2401.00001v1"));
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn arxiv_429_after_retries_keeps_status_in_error() {
        let (base, requests) = spawn_mock_server(vec![
            response_with_headers(429, "rate limited", vec![("Retry-After", "0")]),
            response_with_headers(429, "still limited", vec![("Retry-After", "0")]),
            response_with_headers(429, "still limited", vec![("Retry-After", "0")]),
        ]);
        let provider = ArxivProvider::new(reqwest::Client::builder().build().unwrap());
        let err = provider
            .get_text_with_rate_limit_interval(
                &format!("{base}/api/query?id_list=2401.00001"),
                Duration::ZERO,
            )
            .await
            .expect_err("429 should remain visible after retries");

        assert!(err.to_string().contains("HTTP 429"), "{err}");
        assert_eq!(requests.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn openalex_504_retries_without_key_rotation() {
        let (base, requests) = spawn_mock_server(vec![
            response(504, r#"{"error":"Gateway timeout"}"#),
            response(200, OPENALEX_OK_BODY),
        ]);
        let provider = OpenAlexProvider::new_with_limit(
            reqwest::Client::new(),
            None,
            Some("oa-a,oa-b".to_string()),
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let url = Url::parse(&format!("{base}/works/W1")).unwrap();
        let value = provider.get_json(&url, "openalex").await.unwrap();

        assert_eq!(value["id"], "https://openalex.org/W1");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.contains("api_key=oa-a")),
            "{requests:?}"
        );
    }

    #[tokio::test]
    async fn openalex_429_with_keys_rotates_without_transient_retry() {
        let (base, requests) = spawn_mock_server(vec![
            response(429, r#"{"error":"rate limited"}"#),
            response(200, OPENALEX_OK_BODY),
        ]);
        let provider = OpenAlexProvider::new_with_limit(
            reqwest::Client::new(),
            None,
            Some("oa-a,oa-b".to_string()),
            DEFAULT_MAX_RESPONSE_BYTES,
        );
        let url = Url::parse(&format!("{base}/works/W1")).unwrap();
        let value = provider.get_json(&url, "openalex").await.unwrap();

        assert_eq!(value["id"], "https://openalex.org/W1");
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].contains("api_key=oa-a"), "{requests:?}");
        assert!(requests[1].contains("api_key=oa-b"), "{requests:?}");
    }

    #[tokio::test]
    async fn semantic_request_sends_api_key_header() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}/paper", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).expect("read request");
            let request = String::from_utf8_lossy(&buf[..read]).into_owned();
            let body = r#"{"data":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write response");
            request
        });

        let provider = SemanticProvider::new(reqwest::Client::new(), Some("s2-test".into()));
        provider
            .get_json_with_optional_key(&url, "semantic scholar")
            .await
            .expect("mock semantic response");
        let request = handle.join().expect("server thread");
        assert!(
            request.to_ascii_lowercase().contains("x-api-key: s2-test"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn semantic_rate_limit_serializes_consecutive_requests() {
        let provider = SemanticProvider::new(reqwest::Client::new(), Some("s2-test".into()));
        let start = Instant::now();
        provider.wait_for_rate_limit().await;
        provider.wait_for_rate_limit().await;
        assert!(
            start.elapsed() >= StdDuration::from_millis(1000),
            "second S2 request should be delayed below the 1 rps threshold"
        );
    }

    #[tokio::test]
    async fn provider_rate_limit_serializes_consecutive_requests() {
        let provider = format!("test-provider-{}", unix_millis());
        let start = Instant::now();
        wait_for_global_provider_rate_limit(&provider, Duration::from_millis(80)).await;
        wait_for_global_provider_rate_limit(&provider, Duration::from_millis(80)).await;
        assert!(
            start.elapsed() >= StdDuration::from_millis(70),
            "second provider request should be delayed by the global limiter"
        );
    }
}
