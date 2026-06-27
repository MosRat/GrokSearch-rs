use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use grok_search_types::{GrokSearchError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugEvent {
    pub event: String,
    pub timestamp_unix_ms: u128,
    pub session_id: String,
    pub request_id: String,
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    pub payload: Value,
}

impl DebugEvent {
    pub fn new(event: impl Into<String>, payload: Value) -> Self {
        Self {
            event: event.into(),
            timestamp_unix_ms: now_unix_ms(),
            session_id: "manual".to_string(),
            request_id: Uuid::new_v4().to_string(),
            level: "debug".to_string(),
            operation: None,
            elapsed_ms: None,
            provider: None,
            status: None,
            error: None,
            payload: redact_json_value(payload),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DebugLogger {
    path: Option<Arc<PathBuf>>,
    session_id: String,
}

impl DebugLogger {
    pub fn disabled() -> Self {
        Self {
            path: None,
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.map(Arc::new),
            session_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref().map(PathBuf::as_path)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn request_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    pub fn event(
        &self,
        request_id: &str,
        level: &str,
        event: &str,
        operation: Option<&str>,
        elapsed: Option<Duration>,
        payload: Value,
    ) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let debug_event = DebugEvent {
            event: event.to_string(),
            timestamp_unix_ms: now_unix_ms(),
            session_id: self.session_id.clone(),
            request_id: request_id.to_string(),
            level: level.to_string(),
            operation: operation.map(str::to_string),
            elapsed_ms: elapsed.map(|value| value.as_millis()),
            provider: None,
            status: None,
            error: None,
            payload: redact_json_value(payload),
        };
        let _ = write_jsonl_event(path.as_ref(), &debug_event);
    }

    pub fn error(
        &self,
        request_id: &str,
        event: &str,
        operation: Option<&str>,
        elapsed: Option<Duration>,
        error: &GrokSearchError,
        payload: Value,
    ) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let debug_event = DebugEvent {
            event: event.to_string(),
            timestamp_unix_ms: now_unix_ms(),
            session_id: self.session_id.clone(),
            request_id: request_id.to_string(),
            level: "error".to_string(),
            operation: operation.map(str::to_string),
            elapsed_ms: elapsed.map(|value| value.as_millis()),
            provider: None,
            status: Some(error.kind().to_string()),
            error: Some(error.diagnostics()),
            payload: redact_json_value(payload),
        };
        let _ = write_jsonl_event(path.as_ref(), &debug_event);
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn write_jsonl_event(path: impl AsRef<Path>, event: &DebugEvent) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| GrokSearchError::Io(format!("create log dir failed: {err}")))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| GrokSearchError::Io(format!("open log failed: {err}")))?;
    let line = serde_json::to_string(event)
        .map_err(|err| GrokSearchError::Parse(format!("serialize log failed: {err}")))?;
    writeln!(file, "{line}")
        .map_err(|err| GrokSearchError::Io(format!("write log failed: {err}")))?;
    Ok(())
}

pub fn redact_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_object(map)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_json_value).collect()),
        other => other,
    }
}

fn redact_object(map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter()
        .map(|(key, value)| {
            if is_secret_key(&key) {
                (key, json!("***"))
            } else {
                (key, redact_json_value(value))
            }
        })
        .collect()
}

fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    lowered.contains("authorization")
        || lowered.contains("api_key")
        || lowered.contains("apikey")
        || lowered.contains("token")
        || lowered.contains("secret")
        || lowered.contains("cookie")
        || lowered.contains("password")
}
