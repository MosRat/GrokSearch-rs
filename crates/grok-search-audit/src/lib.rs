use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grok_search_types::{GrokSearchError, Result};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

const AUDIT_STATE: TableDefinition<&str, &[u8]> = TableDefinition::new("audit_state_v1");
const AUDIT_STATE_KEY: &str = "state";
const DEFAULT_RECENT_LIMIT: usize = 1000;

#[derive(Debug, Clone)]
pub struct AuditOptions {
    pub enabled: bool,
    pub path: Option<PathBuf>,
    pub recent_limit: usize,
    pub jsonl_path: Option<PathBuf>,
}

impl AuditOptions {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            path: None,
            recent_limit: DEFAULT_RECENT_LIMIT,
            jsonl_path: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub tool_name: String,
    pub request_id: String,
    pub started_at_unix_ms: u128,
    pub elapsed_ms: u128,
    pub status: AuditStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuditToolStats {
    pub total: u64,
    pub success: u64,
    pub failure: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_unix_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure_unix_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecentCall {
    pub sequence: u64,
    pub tool_name: String,
    pub request_id: String,
    pub started_at_unix_ms: u128,
    pub elapsed_ms: u128,
    pub status: AuditStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    pub enabled: bool,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub recent_limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jsonl_path: Option<String>,
    pub tools: BTreeMap<String, AuditToolStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSnapshot {
    pub summary: AuditSummary,
    pub recent: Vec<AuditRecentCall>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditRecentQuery {
    pub limit: Option<usize>,
    pub tool: Option<String>,
    pub status: Option<AuditStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuditState {
    next_sequence: u64,
    tools: BTreeMap<String, AuditToolStats>,
    recent: Vec<AuditRecentCall>,
}

#[derive(Clone)]
pub struct AuditRecorder {
    session_id: String,
    recent_limit: usize,
    jsonl_path: Option<Arc<PathBuf>>,
    backend: AuditBackend,
}

#[derive(Clone)]
enum AuditBackend {
    Disabled,
    Memory(MemoryAuditStore),
    Redb(Arc<RedbAuditStore>),
}

#[derive(Clone, Default)]
pub struct MemoryAuditStore {
    state: Arc<Mutex<AuditState>>,
}

pub struct RedbAuditStore {
    path: PathBuf,
    database: Arc<Database>,
    lock: Mutex<()>,
}

impl AuditRecorder {
    pub fn new(options: AuditOptions) -> Self {
        Self::from_options(options, true)
    }

    pub fn existing(options: AuditOptions) -> Self {
        Self::from_options(options, false)
    }

    fn from_options(options: AuditOptions, create: bool) -> Self {
        if !options.enabled {
            return Self::disabled();
        }
        let recent_limit = normalize_recent_limit(options.recent_limit);
        let jsonl_path = options.jsonl_path.map(Arc::new);
        let backend = match options.path {
            Some(path) => {
                let opened = if create {
                    RedbAuditStore::open(&path).map(Some)
                } else {
                    RedbAuditStore::open_existing(&path)
                };
                match opened {
                    Ok(Some(store)) => AuditBackend::Redb(Arc::new(store)),
                    Ok(None) => AuditBackend::Memory(MemoryAuditStore::default()),
                    Err(err) => {
                        tracing::warn!(
                            target: "grok_search",
                            error = %err,
                            path = %path.display(),
                            "audit store open failed; falling back to memory"
                        );
                        AuditBackend::Memory(MemoryAuditStore::default())
                    }
                }
            }
            None => AuditBackend::Memory(MemoryAuditStore::default()),
        };
        Self {
            session_id: Uuid::new_v4().to_string(),
            recent_limit,
            jsonl_path,
            backend,
        }
    }

    pub fn disabled() -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            recent_limit: DEFAULT_RECENT_LIMIT,
            jsonl_path: None,
            backend: AuditBackend::Disabled,
        }
    }

    pub fn memory(recent_limit: usize) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            recent_limit: normalize_recent_limit(recent_limit),
            jsonl_path: None,
            backend: AuditBackend::Memory(MemoryAuditStore::default()),
        }
    }

    pub fn enabled(&self) -> bool {
        !matches!(self.backend, AuditBackend::Disabled)
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            AuditBackend::Disabled => "disabled",
            AuditBackend::Memory(_) => "memory",
            AuditBackend::Redb(_) => "redb",
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.backend {
            AuditBackend::Redb(store) => Some(store.path()),
            _ => None,
        }
    }

    pub fn jsonl_path(&self) -> Option<&Path> {
        self.jsonl_path.as_deref().map(PathBuf::as_path)
    }

    pub fn recent_limit(&self) -> usize {
        self.recent_limit
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn request_id(&self) -> String {
        Uuid::new_v4().to_string()
    }

    pub fn record_tool_call(
        &self,
        tool_name: &str,
        request_id: &str,
        started_at_unix_ms: u128,
        elapsed: Duration,
        status: AuditStatus,
        error_kind: Option<&str>,
        payload: Value,
    ) {
        if !self.enabled() {
            return;
        }
        let event = AuditEvent {
            tool_name: tool_name.to_string(),
            request_id: request_id.to_string(),
            started_at_unix_ms,
            elapsed_ms: elapsed.as_millis(),
            status,
            error_kind: error_kind.map(str::to_string),
            payload: redact_json_value(payload),
        };
        match &self.backend {
            AuditBackend::Disabled => {}
            AuditBackend::Memory(store) => store.record(event.clone(), self.recent_limit),
            AuditBackend::Redb(store) => {
                if let Err(err) = store.record(event.clone(), self.recent_limit) {
                    tracing::warn!(
                        target: "grok_search",
                        error = %err,
                        "audit event write failed"
                    );
                }
            }
        }
        if let Some(path) = self.jsonl_path.as_ref() {
            if let Err(err) = write_jsonl_event(path.as_ref(), &event) {
                tracing::warn!(
                    target: "grok_search",
                    error = %err,
                    path = %path.display(),
                    "audit JSONL write failed"
                );
            }
        }
    }

    pub fn snapshot(&self, query: AuditRecentQuery) -> AuditSnapshot {
        let state = self.load_state();
        AuditSnapshot {
            summary: self.summary_from_state(&state),
            recent: filter_recent(&state.recent, query, self.recent_limit),
        }
    }

    pub fn summary(&self) -> AuditSummary {
        self.summary_from_state(&self.load_state())
    }

    pub fn recent(&self, query: AuditRecentQuery) -> Vec<AuditRecentCall> {
        filter_recent(&self.load_state().recent, query, self.recent_limit)
    }

    pub fn clear(&self) -> Result<()> {
        match &self.backend {
            AuditBackend::Disabled => Ok(()),
            AuditBackend::Memory(store) => {
                store.clear();
                Ok(())
            }
            AuditBackend::Redb(store) => store.clear(),
        }
    }

    pub fn diagnostics(&self) -> Value {
        serde_json::to_value(self.summary())
            .unwrap_or_else(|_| json!({ "enabled": self.enabled() }))
    }

    fn load_state(&self) -> AuditState {
        match &self.backend {
            AuditBackend::Disabled => AuditState::default(),
            AuditBackend::Memory(store) => store.load(),
            AuditBackend::Redb(store) => store.load().unwrap_or_else(|err| {
                tracing::warn!(
                    target: "grok_search",
                    error = %err,
                    "audit state read failed"
                );
                AuditState::default()
            }),
        }
    }

    fn summary_from_state(&self, state: &AuditState) -> AuditSummary {
        AuditSummary {
            enabled: self.enabled(),
            backend: self.backend_name().to_string(),
            path: self.path().map(|path| path.display().to_string()),
            recent_limit: self.recent_limit,
            jsonl_path: self.jsonl_path().map(|path| path.display().to_string()),
            tools: state.tools.clone(),
        }
    }
}

impl MemoryAuditStore {
    fn record(&self, event: AuditEvent, recent_limit: usize) {
        if let Ok(mut state) = self.state.lock() {
            apply_event(&mut state, event, recent_limit);
        }
    }

    fn load(&self) -> AuditState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = AuditState::default();
        }
    }
}

impl RedbAuditStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|err| {
                GrokSearchError::Io(format!(
                    "create audit directory {}: {err}",
                    parent.display()
                ))
            })?;
        }
        let database = if path.exists() {
            Database::open(&path)
        } else {
            Database::create(&path)
        }
        .map_err(|err| GrokSearchError::Io(format!("open audit db {}: {err}", path.display())))?;
        Ok(Self {
            path,
            database: Arc::new(database),
            lock: Mutex::new(()),
        })
    }

    pub fn open_existing(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(None);
        }
        let database = Database::open(&path).map_err(|err| {
            GrokSearchError::Io(format!("open audit db {}: {err}", path.display()))
        })?;
        Ok(Some(Self {
            path,
            database: Arc::new(database),
            lock: Mutex::new(()),
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn record(&self, event: AuditEvent, recent_limit: usize) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| GrokSearchError::Io("audit lock poisoned".to_string()))?;
        let txn = self
            .database
            .begin_write()
            .map_err(audit_err("begin write"))?;
        {
            let mut table = txn
                .open_table(AUDIT_STATE)
                .map_err(audit_err("open state"))?;
            let mut state = match table
                .get(AUDIT_STATE_KEY)
                .map_err(audit_err("read state"))?
            {
                Some(bytes) => serde_json::from_slice(bytes.value())
                    .map_err(|err| GrokSearchError::Parse(format!("parse audit state: {err}")))?,
                None => AuditState::default(),
            };
            apply_event(&mut state, event, recent_limit);
            let bytes = serde_json::to_vec(&state)
                .map_err(|err| GrokSearchError::Parse(format!("serialize audit state: {err}")))?;
            table
                .insert(AUDIT_STATE_KEY, bytes.as_slice())
                .map_err(audit_err("write state"))?;
        }
        txn.commit().map_err(audit_err("commit write"))?;
        Ok(())
    }

    fn load(&self) -> Result<AuditState> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| GrokSearchError::Io("audit lock poisoned".to_string()))?;
        self.load_unlocked()
    }

    fn clear(&self) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| GrokSearchError::Io("audit lock poisoned".to_string()))?;
        self.save_unlocked(&AuditState::default())
    }

    fn load_unlocked(&self) -> Result<AuditState> {
        let txn = self
            .database
            .begin_read()
            .map_err(audit_err("begin read"))?;
        let table = match txn.open_table(AUDIT_STATE) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(AuditState::default()),
            Err(err) => return Err(audit_err("open state")(err)),
        };
        let Some(bytes) = table
            .get(AUDIT_STATE_KEY)
            .map_err(audit_err("read state"))?
        else {
            return Ok(AuditState::default());
        };
        serde_json::from_slice(bytes.value())
            .map_err(|err| GrokSearchError::Parse(format!("parse audit state: {err}")))
    }

    fn save_unlocked(&self, state: &AuditState) -> Result<()> {
        let bytes = serde_json::to_vec(state)
            .map_err(|err| GrokSearchError::Parse(format!("serialize audit state: {err}")))?;
        let txn = self
            .database
            .begin_write()
            .map_err(audit_err("begin write"))?;
        {
            let mut table = txn
                .open_table(AUDIT_STATE)
                .map_err(audit_err("open state"))?;
            table
                .insert(AUDIT_STATE_KEY, bytes.as_slice())
                .map_err(audit_err("write state"))?;
        }
        txn.commit().map_err(audit_err("commit write"))?;
        Ok(())
    }
}

fn apply_event(state: &mut AuditState, event: AuditEvent, recent_limit: usize) {
    let stats = state.tools.entry(event.tool_name.clone()).or_default();
    stats.total = stats.total.saturating_add(1);
    match event.status {
        AuditStatus::Success => {
            stats.success = stats.success.saturating_add(1);
            stats.last_success_unix_ms = Some(event.started_at_unix_ms);
        }
        AuditStatus::Error => {
            stats.failure = stats.failure.saturating_add(1);
            stats.last_failure_unix_ms = Some(event.started_at_unix_ms);
            stats.last_error_kind = event.error_kind.clone();
        }
    }

    let sequence = state.next_sequence;
    state.next_sequence = state.next_sequence.saturating_add(1);
    state.recent.push(AuditRecentCall {
        sequence,
        tool_name: event.tool_name,
        request_id: event.request_id,
        started_at_unix_ms: event.started_at_unix_ms,
        elapsed_ms: event.elapsed_ms,
        status: event.status,
        error_kind: event.error_kind,
        payload: event.payload,
    });
    let limit = normalize_recent_limit(recent_limit);
    if state.recent.len() > limit {
        let overflow = state.recent.len() - limit;
        state.recent.drain(0..overflow);
    }
}

fn filter_recent(
    recent: &[AuditRecentCall],
    query: AuditRecentQuery,
    default_limit: usize,
) -> Vec<AuditRecentCall> {
    let limit = query.limit.unwrap_or(default_limit).min(default_limit);
    recent
        .iter()
        .rev()
        .filter(|item| match query.tool.as_ref() {
            Some(tool) => item.tool_name == *tool,
            None => true,
        })
        .filter(|item| match query.status {
            Some(status) => item.status == status,
            None => true,
        })
        .take(limit)
        .cloned()
        .collect()
}

pub fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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
    let normalized: String = lowered
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    lowered.contains("authorization")
        || lowered.contains("api_key")
        || lowered.contains("apikey")
        || normalized.contains("apikey")
        || normalized.contains("accesskey")
        || lowered.contains("token")
        || lowered.contains("secret")
        || lowered.contains("cookie")
        || lowered.contains("password")
}

pub fn write_jsonl_event(path: impl AsRef<Path>, event: &AuditEvent) -> Result<()> {
    let path = path.as_ref();
    ensure_parent_dir(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| GrokSearchError::Io(format!("open audit log failed: {err}")))?;
    let mut event = event.clone();
    event.payload = redact_json_value(event.payload);
    let line = serde_json::to_string(&event)
        .map_err(|err| GrokSearchError::Parse(format!("serialize audit log failed: {err}")))?;
    writeln!(file, "{line}")
        .map_err(|err| GrokSearchError::Io(format!("write audit log failed: {err}")))?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|err| GrokSearchError::Io(format!("create audit log dir failed: {err}")))?;
    }
    Ok(())
}

fn normalize_recent_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_RECENT_LIMIT
    } else {
        limit
    }
}

fn audit_err<E: std::fmt::Display>(context: &'static str) -> impl FnOnce(E) -> GrokSearchError {
    move |err| GrokSearchError::Io(format!("audit {context}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(tool: &str, status: AuditStatus, idx: usize) -> AuditEvent {
        AuditEvent {
            tool_name: tool.to_string(),
            request_id: format!("req-{idx}"),
            started_at_unix_ms: idx as u128,
            elapsed_ms: 10,
            status,
            error_kind: (status == AuditStatus::Error).then(|| "provider".to_string()),
            payload: json!({ "idx": idx, "api_key": "secret" }),
        }
    }

    #[test]
    fn redaction_covers_secret_like_keys() {
        let value = json!({
            "Authorization": "Bearer secret",
            "api_key": "secret",
            "x-api-key": "secret",
            "access-key": "secret",
            "token": "secret",
            "secret": "secret",
            "cookie": "secret",
            "password": "secret",
            "safe": "visible"
        });
        let redacted = redact_json_value(value);
        let text = serde_json::to_string(&redacted).unwrap();
        assert!(text.contains("visible"));
        assert!(!text.contains("Bearer secret"));
        assert!(!text.contains(":\"secret\""));
    }

    #[test]
    fn memory_store_counts_and_prunes_recent_calls() {
        let recorder = AuditRecorder::memory(2);
        recorder.record_tool_call(
            "web_search",
            "a",
            1,
            Duration::from_millis(1),
            AuditStatus::Success,
            None,
            json!({}),
        );
        recorder.record_tool_call(
            "web_search",
            "b",
            2,
            Duration::from_millis(1),
            AuditStatus::Error,
            Some("provider"),
            json!({}),
        );
        recorder.record_tool_call(
            "web_fetch",
            "c",
            3,
            Duration::from_millis(1),
            AuditStatus::Success,
            None,
            json!({}),
        );

        let snapshot = recorder.snapshot(AuditRecentQuery::default());
        assert_eq!(snapshot.summary.tools["web_search"].total, 2);
        assert_eq!(snapshot.summary.tools["web_search"].failure, 1);
        assert_eq!(snapshot.recent.len(), 2);
        assert_eq!(snapshot.recent[0].request_id, "c");
        assert_eq!(snapshot.recent[1].request_id, "b");
    }

    #[test]
    fn redb_store_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.redb");
        {
            let store = RedbAuditStore::open(&path).unwrap();
            store
                .record(event("web_search", AuditStatus::Success, 1), 10)
                .unwrap();
            store
                .record(event("web_search", AuditStatus::Error, 2), 10)
                .unwrap();
        }
        let store = RedbAuditStore::open(&path).unwrap();
        let state = store.load().unwrap();
        assert_eq!(state.tools["web_search"].total, 2);
        assert_eq!(state.tools["web_search"].success, 1);
        assert_eq!(state.tools["web_search"].failure, 1);
        assert_eq!(state.recent.len(), 2);
    }

    #[test]
    fn existing_recorder_does_not_create_missing_redb_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.redb");
        let recorder = AuditRecorder::existing(AuditOptions {
            enabled: true,
            path: Some(path.clone()),
            recent_limit: 1000,
            jsonl_path: None,
        });

        assert_eq!(recorder.backend_name(), "memory");
        assert!(!path.exists());
        assert!(recorder.summary().tools.is_empty());
    }

    #[test]
    fn jsonl_output_writes_redacted_event() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let event = event("web_search", AuditStatus::Success, 1);
        write_jsonl_event(&path, &event).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("web_search"));
        assert!(!text.contains("secret"));
        assert!(text.contains("***"));
    }

    #[test]
    fn jsonl_output_accepts_plain_relative_filename() {
        ensure_parent_dir(Path::new("audit.jsonl")).unwrap();
    }
}
