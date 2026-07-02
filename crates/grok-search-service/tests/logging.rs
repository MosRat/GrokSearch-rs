use std::time::Duration;

use grok_search_audit::{redact_json_value, AuditRecentQuery, AuditRecorder, AuditStatus};
use grok_search_service::{SearchService, SourceProvider};
use grok_search_types::model::search::SearchFilters;
use grok_search_types::model::source::Source;
use grok_search_types::model::tool::WebSearchInput;
use grok_search_types::{AcademicSearchInput, Result};
use tempfile::tempdir;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn debug_log_redacts_authorization_and_api_keys() {
    let value = serde_json::json!({
        "Authorization": "Bearer secret-token",
        "GROK_SEARCH_API_KEY": "grok-secret",
        "cookie": "session=secret",
        "nested": { "password": "secret-password" },
        "tools": [{"type": "web_search"}]
    });

    let redacted = redact_json_value(value);
    let text = serde_json::to_string(&redacted).unwrap();

    assert!(text.contains("web_search"));
    assert!(!text.contains("secret-token"));
    assert!(!text.contains("grok-secret"));
    assert!(!text.contains("secret-password"));
    assert!(!text.contains("session=secret"));
}

#[test]
fn debug_logger_writes_jsonl_with_metadata_and_redaction() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("audit.jsonl");
    let recorder = AuditRecorder::new(grok_search_audit::AuditOptions {
        enabled: true,
        path: None,
        recent_limit: 1000,
        jsonl_path: Some(path.clone()),
    });
    let request_id = recorder.request_id();

    recorder.record_tool_call(
        "web_fetch",
        &request_id,
        1,
        Duration::from_millis(12),
        AuditStatus::Success,
        None,
        serde_json::json!({
            "url": { "host": "example.com", "path": "/docs" },
            "api_key": "secret",
            "response_bytes": 123
        }),
    );

    let text = std::fs::read_to_string(path).unwrap();
    let line: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(line["tool_name"], "web_fetch");
    assert_eq!(line["request_id"], request_id);
    assert_eq!(line["elapsed_ms"], 12);
    assert_eq!(line["payload"]["response_bytes"], 123);
    assert_eq!(line["payload"]["api_key"], "***");
    assert!(!text.contains("secret"));
}

#[tokio::test]
async fn doctor_verbose_reports_runtime_and_jsonl_logging() {
    let _guard = ENV_LOCK.lock().unwrap();
    let previous_filter = std::env::var("GROK_SEARCH_LOG_EFFECTIVE_FILTER").ok();
    let previous_source = std::env::var("GROK_SEARCH_LOG_FILTER_SOURCE").ok();
    let previous_explicit = std::env::var("GROK_SEARCH_LOG_EXPLICIT").ok();
    std::env::set_var("GROK_SEARCH_LOG_EFFECTIVE_FILTER", "warn");
    std::env::set_var("GROK_SEARCH_LOG_FILTER_SOURCE", "default");
    std::env::set_var("GROK_SEARCH_LOG_EXPLICIT", "false");

    let service = SearchService::fake_with_sources();
    let report = service.doctor_with_options(true).await;

    assert_eq!(report["diagnostics"]["runtime_log"]["filter"], "warn");
    assert_eq!(report["diagnostics"]["runtime_log"]["source"], "default");
    assert_eq!(report["diagnostics"]["runtime_log"]["explicit"], false);
    assert_eq!(report["diagnostics"]["runtime_log"]["stream"], "stderr");
    assert_eq!(report["diagnostics"]["audit"]["enabled"], true);
    assert_eq!(report["diagnostics"]["audit"]["backend"], "memory");
    assert_eq!(report["diagnostics"]["debug_log"]["deprecated"], true);

    restore_env("GROK_SEARCH_LOG_EFFECTIVE_FILTER", previous_filter);
    restore_env("GROK_SEARCH_LOG_FILTER_SOURCE", previous_source);
    restore_env("GROK_SEARCH_LOG_EXPLICIT", previous_explicit);
}

fn restore_env(key: &str, value: Option<String>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

#[tokio::test]
async fn audit_summary_counts_successful_tool_calls() {
    let service = SearchService::fake_with_sources();
    service
        .web_search(WebSearchInput {
            query: "rust tracing audit".to_string(),
            extra_sources: Some(1),
            ..WebSearchInput::default()
        })
        .await
        .expect("fake web_search should succeed");

    let summary = service.audit_summary();
    let stats = summary.tools.get("web_search").expect("web_search stats");
    assert_eq!(stats.total, 1);
    assert_eq!(stats.success, 1);
    assert_eq!(stats.failure, 0);
}

#[tokio::test]
async fn audit_summary_counts_failed_tool_calls() {
    let service = SearchService::fake_with_sources();
    service
        .web_fetch("not-a-url", None)
        .await
        .expect_err("invalid URL should fail");

    let summary = service.audit_summary();
    let stats = summary.tools.get("web_fetch").expect("web_fetch stats");
    assert_eq!(stats.total, 1);
    assert_eq!(stats.success, 0);
    assert_eq!(stats.failure, 1);
    assert_eq!(stats.last_error_kind.as_deref(), Some("invalid_params"));
}

#[tokio::test]
async fn audit_recent_calls_honor_configured_limit() {
    let service = SearchService::fake_custom(
        None,
        std::sync::Arc::new(TestSourceProvider),
        None,
        [("GROK_SEARCH_AUDIT_RECENT_LIMIT", "2")],
    );

    for url in ["bad-1", "bad-2", "bad-3"] {
        let _ = service.web_fetch(url, None).await;
    }

    let recent = service.audit_recent(AuditRecentQuery::default());
    assert_eq!(recent.len(), 2);
    assert!(recent.iter().all(|call| call.tool_name == "web_fetch"));
}

#[tokio::test]
async fn audit_summary_counts_academic_provider_setup_failures() {
    let service = SearchService::fake_with_sources();
    service
        .academic_search(AcademicSearchInput {
            query: "retrieval augmented generation".to_string(),
            ..AcademicSearchInput::default()
        })
        .await
        .expect_err("fake service has no academic provider");

    let summary = service.audit_summary();
    let stats = summary
        .tools
        .get("academic_search")
        .expect("academic_search stats");
    assert_eq!(stats.total, 1);
    assert_eq!(stats.failure, 1);
    assert_eq!(stats.last_error_kind.as_deref(), Some("missing_config"));
}

struct TestSourceProvider;

#[async_trait::async_trait]
impl SourceProvider for TestSourceProvider {
    async fn search_sources(
        &self,
        _query: &str,
        max_results: usize,
        _filters: &SearchFilters,
    ) -> Result<Vec<Source>> {
        Ok((0..max_results)
            .map(|idx| Source::new(format!("https://example.com/{idx}"), "test"))
            .collect())
    }

    async fn fetch(&self, url: &str) -> Result<String> {
        Ok(format!("Fetched {url}"))
    }

    async fn map(&self, url: &str, max_results: usize) -> Result<Vec<Source>> {
        Ok((0..max_results)
            .map(|idx| Source::new(format!("{url}/{idx}"), "test"))
            .collect())
    }
}
