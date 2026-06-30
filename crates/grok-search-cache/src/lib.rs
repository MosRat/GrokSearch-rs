use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use grok_search_types::{GrokSearchError, Result};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const PROGRESSIVE_ITEMS: TableDefinition<&str, &[u8]> =
    TableDefinition::new("progressive_items_v1");
const PROGRESSIVE_META: TableDefinition<&str, &[u8]> = TableDefinition::new("progressive_meta_v1");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressiveCacheMetadata {
    pub cache_key: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub expires_at_unix: Option<u64>,
    pub size_bytes: u64,
    pub pdf_sha256: String,
    pub input_text_sha256: String,
    pub strategy_hash: String,
    pub model: String,
    pub input_profile: String,
    pub prompt_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressiveCacheEntry {
    pub bytes: Vec<u8>,
    pub metadata: ProgressiveCacheMetadata,
}

#[derive(Debug, Clone)]
pub struct ProgressiveCachePut {
    pub cache_key: String,
    pub bytes: Vec<u8>,
    pub ttl_seconds: Option<u64>,
    pub pdf_sha256: String,
    pub input_text_sha256: String,
    pub strategy_hash: String,
    pub model: String,
    pub input_profile: String,
    pub prompt_profile: String,
}

pub trait ProgressiveCacheStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<ProgressiveCacheEntry>>;
    fn put(&self, put: ProgressiveCachePut, max_entries: usize)
        -> Result<ProgressiveCacheMetadata>;
    fn remove(&self, key: &str) -> Result<bool>;
    fn list_metadata(&self) -> Result<Vec<ProgressiveCacheMetadata>>;
}

#[derive(Clone)]
pub struct RedbProgressiveCache {
    path: PathBuf,
    database: Arc<Database>,
}

impl RedbProgressiveCache {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                GrokSearchError::Io(format!(
                    "create cache directory {}: {err}",
                    parent.display()
                ))
            })?;
        }
        let database = if path.exists() {
            Database::open(&path)
        } else {
            Database::create(&path)
        }
        .map_err(|err| GrokSearchError::Io(format!("open redb cache {}: {err}", path.display())))?;
        Ok(Self {
            path,
            database: Arc::new(database),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ProgressiveCacheStore for RedbProgressiveCache {
    fn get(&self, key: &str) -> Result<Option<ProgressiveCacheEntry>> {
        let txn = self
            .database
            .begin_read()
            .map_err(cache_err("begin read"))?;
        let items = match txn.open_table(PROGRESSIVE_ITEMS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(cache_err("open items")(err)),
        };
        let Some(bytes) = items.get(key).map_err(cache_err("read item"))? else {
            return Ok(None);
        };
        let bytes = bytes.value().to_vec();
        drop(items);

        let meta = match txn.open_table(PROGRESSIVE_META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(err) => return Err(cache_err("open metadata")(err)),
        };
        let Some(meta_bytes) = meta.get(key).map_err(cache_err("read metadata"))? else {
            return Ok(None);
        };
        let metadata = serde_json::from_slice::<ProgressiveCacheMetadata>(meta_bytes.value())
            .map_err(|err| {
                GrokSearchError::Parse(format!("parse progressive cache metadata: {err}"))
            })?;
        if metadata
            .expires_at_unix
            .is_some_and(|expires_at| expires_at <= unix_now())
        {
            return Ok(None);
        }
        Ok(Some(ProgressiveCacheEntry { bytes, metadata }))
    }

    fn put(
        &self,
        put: ProgressiveCachePut,
        max_entries: usize,
    ) -> Result<ProgressiveCacheMetadata> {
        let now = unix_now();
        let existing_created_at = self
            .get(&put.cache_key)
            .ok()
            .flatten()
            .map(|entry| entry.metadata.created_at_unix)
            .unwrap_or(now);
        let metadata = ProgressiveCacheMetadata {
            cache_key: put.cache_key.clone(),
            created_at_unix: existing_created_at,
            updated_at_unix: now,
            expires_at_unix: put.ttl_seconds.map(|ttl| now.saturating_add(ttl)),
            size_bytes: put.bytes.len() as u64,
            pdf_sha256: put.pdf_sha256,
            input_text_sha256: put.input_text_sha256,
            strategy_hash: put.strategy_hash,
            model: put.model,
            input_profile: put.input_profile,
            prompt_profile: put.prompt_profile,
        };
        let meta_bytes = serde_json::to_vec(&metadata)
            .map_err(|err| GrokSearchError::Parse(format!("serialize cache metadata: {err}")))?;

        let txn = self
            .database
            .begin_write()
            .map_err(cache_err("begin write"))?;
        {
            let mut items = txn
                .open_table(PROGRESSIVE_ITEMS)
                .map_err(cache_err("open items"))?;
            items
                .insert(put.cache_key.as_str(), put.bytes.as_slice())
                .map_err(cache_err("write item"))?;
        }
        {
            let mut meta = txn
                .open_table(PROGRESSIVE_META)
                .map_err(cache_err("open metadata"))?;
            meta.insert(put.cache_key.as_str(), meta_bytes.as_slice())
                .map_err(cache_err("write metadata"))?;
        }
        txn.commit().map_err(cache_err("commit write"))?;
        self.prune(max_entries)?;
        Ok(metadata)
    }

    fn remove(&self, key: &str) -> Result<bool> {
        let txn = self
            .database
            .begin_write()
            .map_err(cache_err("begin write"))?;
        let removed = {
            let mut removed = false;
            if let Ok(mut items) = txn.open_table(PROGRESSIVE_ITEMS) {
                removed |= items
                    .remove(key)
                    .map_err(cache_err("remove item"))?
                    .is_some();
            }
            if let Ok(mut meta) = txn.open_table(PROGRESSIVE_META) {
                removed |= meta
                    .remove(key)
                    .map_err(cache_err("remove metadata"))?
                    .is_some();
            }
            removed
        };
        txn.commit().map_err(cache_err("commit remove"))?;
        Ok(removed)
    }

    fn list_metadata(&self) -> Result<Vec<ProgressiveCacheMetadata>> {
        let txn = self
            .database
            .begin_read()
            .map_err(cache_err("begin read"))?;
        let meta = match txn.open_table(PROGRESSIVE_META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(err) => return Err(cache_err("open metadata")(err)),
        };
        let mut out = Vec::new();
        for row in meta.iter().map_err(cache_err("scan metadata"))? {
            let (_, value) = row.map_err(cache_err("read metadata row"))?;
            match serde_json::from_slice::<ProgressiveCacheMetadata>(value.value()) {
                Ok(metadata) => out.push(metadata),
                Err(err) => {
                    return Err(GrokSearchError::Parse(format!(
                        "parse progressive cache metadata row: {err}"
                    )))
                }
            }
        }
        Ok(out)
    }
}

impl RedbProgressiveCache {
    fn prune(&self, max_entries: usize) -> Result<()> {
        let now = unix_now();
        let mut metadata = self.list_metadata()?;
        let mut remove_keys = metadata
            .iter()
            .filter(|item| {
                item.expires_at_unix
                    .is_some_and(|expires_at| expires_at <= now)
            })
            .map(|item| item.cache_key.clone())
            .collect::<Vec<_>>();
        if max_entries > 0 {
            metadata.retain(|item| !remove_keys.iter().any(|key| key == &item.cache_key));
            metadata.sort_by_key(|item| item.updated_at_unix);
            let overflow = metadata.len().saturating_sub(max_entries);
            remove_keys.extend(
                metadata
                    .into_iter()
                    .take(overflow)
                    .map(|item| item.cache_key),
            );
        }
        for key in remove_keys {
            let _ = self.remove(&key)?;
        }
        Ok(())
    }
}

fn cache_err<E: std::fmt::Display>(context: &'static str) -> impl FnOnce(E) -> GrokSearchError {
    move |err| GrokSearchError::Io(format!("progressive cache {context}: {err}"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(key: &str) -> ProgressiveCachePut {
        ProgressiveCachePut {
            cache_key: key.to_string(),
            bytes: br#"{"ok":true}"#.to_vec(),
            ttl_seconds: Some(60),
            pdf_sha256: "pdf".to_string(),
            input_text_sha256: "text".to_string(),
            strategy_hash: "strategy".to_string(),
            model: "model".to_string(),
            input_profile: "input".to_string(),
            prompt_profile: "prompt".to_string(),
        }
    }

    #[test]
    fn redb_progressive_cache_round_trips_and_prunes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let cache = RedbProgressiveCache::open(dir.path().join("cache.redb")).expect("open cache");
        cache.put(put("a"), 2).expect("put a");
        cache.put(put("b"), 2).expect("put b");
        cache.put(put("c"), 2).expect("put c");
        assert!(cache.get("a").expect("get a").is_none());
        assert_eq!(
            cache.get("c").expect("get c").expect("entry").bytes,
            br#"{"ok":true}"#.to_vec()
        );
        assert_eq!(cache.list_metadata().expect("metadata").len(), 2);
    }
}
