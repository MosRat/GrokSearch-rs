use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grok_search_net::http::{build_client, get_json_limited, DEFAULT_MAX_RESPONSE_BYTES};
use grok_search_provider_core::ZhihuProvider;
use grok_search_types::{
    GrokSearchError, Result, ZhihuSearchInput, ZhihuSearchItem, ZhihuSearchOutput,
};
use reqwest::header::{HeaderName, AUTHORIZATION};
use reqwest::Client;
use serde_json::Value;
use url::Url;

const DEFAULT_BASE_URL: &str = "https://developer.zhihu.com";
const DEFAULT_SEARCH_PATH: &str = "/api/v1/content/zhihu_search";

#[derive(Clone)]
pub struct ZhihuSearchProvider {
    client: Client,
    api_key: String,
    base_url: String,
    search_url: Option<String>,
    max_response_bytes: usize,
}

impl ZhihuSearchProvider {
    pub fn try_new(api_key: impl Into<String>, timeout: Duration) -> Result<Self> {
        Ok(Self::with_client(build_client(timeout)?, api_key))
    }

    pub fn new(api_key: impl Into<String>, timeout: Duration) -> Self {
        Self::try_new(api_key, timeout).expect("build HTTP client for ZhihuSearchProvider")
    }

    pub fn with_client(client: Client, api_key: impl Into<String>) -> Self {
        Self::with_client_base_url_and_limit(
            client,
            api_key,
            DEFAULT_BASE_URL,
            None,
            DEFAULT_MAX_RESPONSE_BYTES,
        )
    }

    pub fn with_client_base_url_and_limit(
        client: Client,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        search_url: Option<String>,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            search_url,
            max_response_bytes,
        }
    }

    pub async fn search(&self, input: ZhihuSearchInput) -> Result<ZhihuSearchOutput> {
        let query = input.query.trim().to_string();
        if query.is_empty() {
            return Err(GrokSearchError::InvalidParams(
                "zhihu_search.query is required".to_string(),
            ));
        }
        let count = input.count.unwrap_or(10).clamp(1, 10);
        let url = self.search_endpoint(&query, count)?;
        let auth = format!("Bearer {}", self.api_key);
        let timestamp = unix_timestamp().to_string();
        let headers = [
            (AUTHORIZATION, auth.as_str()),
            (
                HeaderName::from_static("x-request-timestamp"),
                timestamp.as_str(),
            ),
        ];
        let raw = get_json_limited(
            &self.client,
            url.as_str(),
            &headers,
            "Zhihu",
            self.max_response_bytes,
        )
        .await?;
        Ok(normalize_zhihu_search_response(query, &raw))
    }

    fn search_endpoint(&self, query: &str, count: usize) -> Result<Url> {
        let endpoint = self
            .search_url
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}{}", self.base_url, DEFAULT_SEARCH_PATH));
        let mut url = Url::parse(&endpoint).map_err(|err| {
            GrokSearchError::Config(format!("invalid Zhihu search endpoint: {err}"))
        })?;
        url.query_pairs_mut()
            .append_pair("Query", query)
            .append_pair("Count", &count.to_string());
        Ok(url)
    }
}

#[async_trait::async_trait]
impl ZhihuProvider for ZhihuSearchProvider {
    async fn search(&self, input: ZhihuSearchInput) -> Result<ZhihuSearchOutput> {
        ZhihuSearchProvider::search(self, input).await
    }
}

pub fn normalize_zhihu_search_response(query: String, raw: &Value) -> ZhihuSearchOutput {
    let data = raw.get("Data").and_then(Value::as_object);
    let items = data
        .and_then(|data| data.get("Items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_zhihu_item)
        .collect::<Vec<_>>();
    ZhihuSearchOutput {
        query,
        code: int_value(raw.get("Code")).unwrap_or(-1),
        message: string_value(raw.get("Message")),
        item_count: items.len(),
        items,
    }
}

fn parse_zhihu_item(value: &Value) -> Option<ZhihuSearchItem> {
    let item = value.as_object()?;
    Some(ZhihuSearchItem {
        title: string_value(item.get("Title")),
        url: string_value(item.get("Url")),
        author_name: string_value(item.get("AuthorName")),
        summary: string_value(item.get("ContentText")),
        vote_up_count: unsigned_value(item.get("VoteUpCount")).unwrap_or(0),
        comment_count: unsigned_value(item.get("CommentCount")).unwrap_or(0),
        edit_time: int_value(item.get("EditTime")).unwrap_or(0),
    })
}

fn string_value(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn unsigned_value(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).or_else(|| {
        value
            .and_then(Value::as_i64)
            .and_then(|v| u64::try_from(v).ok())
    })
}

fn int_value(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).or_else(|| {
        value
            .and_then(Value::as_u64)
            .and_then(|v| i64::try_from(v).ok())
    })
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    const OK_BODY: &str = r#"{
        "Code": 0,
        "Message": "success",
        "Data": {
            "Items": [
                {
                    "Title": "如何理解 rave 文化",
                    "Url": "https://www.zhihu.com/question/1/answer/2",
                    "AuthorName": "测试作者",
                    "ContentText": "这是一段摘要",
                    "VoteUpCount": 42,
                    "CommentCount": 7,
                    "EditTime": 1710000000
                }
            ]
        }
    }"#;

    #[test]
    fn normalizes_zhihu_search_response() {
        let raw: Value = serde_json::from_str(OK_BODY).unwrap();
        let output = normalize_zhihu_search_response("rave".to_string(), &raw);
        assert_eq!(output.query, "rave");
        assert_eq!(output.code, 0);
        assert_eq!(output.message, "success");
        assert_eq!(output.item_count, 1);
        assert_eq!(output.items[0].title, "如何理解 rave 文化");
        assert_eq!(output.items[0].summary, "这是一段摘要");
        assert_eq!(output.items[0].author_name, "测试作者");
        assert_eq!(output.items[0].vote_up_count, 42);
        assert_eq!(output.items[0].comment_count, 7);
        assert_eq!(output.items[0].edit_time, 1710000000);
    }

    #[test]
    fn missing_items_normalizes_to_empty_list() {
        let raw: Value = serde_json::json!({ "Code": 0, "Message": "ok", "Data": {} });
        let output = normalize_zhihu_search_response("empty".to_string(), &raw);
        assert_eq!(output.item_count, 0);
        assert!(output.items.is_empty());
    }

    #[tokio::test]
    async fn search_sends_query_count_and_bearer_headers() {
        let seen = Arc::new(Mutex::new(String::new()));
        let base = spawn_mock_server(200, OK_BODY, Arc::clone(&seen));
        let provider = ZhihuSearchProvider::with_client_base_url_and_limit(
            reqwest::Client::new(),
            "secret-key",
            base,
            None,
            DEFAULT_MAX_RESPONSE_BYTES,
        );

        let output = provider
            .search(ZhihuSearchInput {
                query: "  如何理解 rave 文化  ".to_string(),
                count: Some(5),
            })
            .await
            .expect("zhihu search");

        assert_eq!(output.query, "如何理解 rave 文化");
        let request = seen.lock().expect("seen lock").clone();
        assert!(request.starts_with("get /api/v1/content/zhihu_search?"));
        assert!(request.contains("query=%e5%a6%82%e4%bd%95"));
        assert!(request.contains("count=5"));
        assert!(request.contains("authorization: bearer secret-key"));
        assert!(request.contains("x-request-timestamp:"));
    }

    #[tokio::test]
    async fn search_returns_http_error_with_body_context() {
        let seen = Arc::new(Mutex::new(String::new()));
        let base = spawn_mock_server(403, "Forbidden", seen);
        let provider = ZhihuSearchProvider::with_client_base_url_and_limit(
            reqwest::Client::new(),
            "secret-key",
            base,
            None,
            DEFAULT_MAX_RESPONSE_BYTES,
        );

        let err = provider
            .search(ZhihuSearchInput {
                query: "test".to_string(),
                count: Some(1),
            })
            .await
            .expect_err("403 should fail");

        let text = err.to_string();
        assert!(text.contains("HTTP 403"), "{text}");
        assert!(text.contains("Forbidden"), "{text}");
    }

    #[tokio::test]
    async fn search_returns_parse_error_for_non_json() {
        let seen = Arc::new(Mutex::new(String::new()));
        let base = spawn_mock_server(200, "not json", seen);
        let provider = ZhihuSearchProvider::with_client_base_url_and_limit(
            reqwest::Client::new(),
            "secret-key",
            base,
            None,
            DEFAULT_MAX_RESPONSE_BYTES,
        );

        let err = provider
            .search(ZhihuSearchInput {
                query: "test".to_string(),
                count: Some(1),
            })
            .await
            .expect_err("non-json should fail");

        assert!(err.to_string().contains("invalid Zhihu JSON"));
    }

    fn spawn_mock_server(
        status: u16,
        body: &'static str,
        seen_request: Arc<Mutex<String>>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://{}", listener.local_addr().expect("local addr"));
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = stream.read(&mut buf).expect("read request");
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
                if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            *seen_request.lock().expect("seen request lock") =
                String::from_utf8_lossy(&raw).to_ascii_lowercase();

            let reason = match status {
                200 => "OK",
                403 => "Forbidden",
                _ => "Mock",
            };
            let content_type = if body.starts_with('{') {
                "application/json"
            } else {
                "text/plain"
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });
        base
    }
}
