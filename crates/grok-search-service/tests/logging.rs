use std::time::Duration;

use grok_search_service::logging::{redact_json_value, DebugLogger};
use tempfile::tempdir;

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
    let path = dir.path().join("debug.jsonl");
    let logger = DebugLogger::new(Some(path.clone()));
    let request_id = logger.request_id();

    logger.event(
        &request_id,
        "debug",
        "web_fetch.success",
        Some("web_fetch"),
        Some(Duration::from_millis(12)),
        serde_json::json!({
            "url": { "host": "example.com", "path": "/docs" },
            "api_key": "secret",
            "response_bytes": 123
        }),
    );

    let text = std::fs::read_to_string(path).unwrap();
    let line: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
    assert_eq!(line["event"], "web_fetch.success");
    assert_eq!(line["operation"], "web_fetch");
    assert_eq!(line["request_id"], request_id);
    assert_eq!(line["elapsed_ms"], 12);
    assert_eq!(line["payload"]["response_bytes"], 123);
    assert_eq!(line["payload"]["api_key"], "***");
    assert!(!text.contains("secret"));
}
