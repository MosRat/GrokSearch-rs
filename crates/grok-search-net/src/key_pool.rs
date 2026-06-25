use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Round-robin pool over one or more provider API keys. Shared across provider
/// clones so the rotation cursor is global and credit usage spreads evenly.
#[derive(Clone)]
pub struct KeyPool {
    inner: Arc<KeyPoolInner>,
}

struct KeyPoolInner {
    keys: Vec<String>,
    cursor: AtomicUsize,
}

impl KeyPool {
    /// Split a comma-separated key list into the pool. Whitespace around each
    /// segment is trimmed and empty segments are dropped. When no non-empty
    /// segment remains, keep the raw value as one key to preserve legacy
    /// single-key failure behavior.
    pub fn parse(raw: &str) -> Self {
        let mut keys: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect();
        if keys.is_empty() {
            keys.push(raw.to_string());
        }
        Self {
            inner: Arc::new(KeyPoolInner {
                keys,
                cursor: AtomicUsize::new(0),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.keys.is_empty()
    }

    /// Index the next request should start from. `Relaxed` is enough: the
    /// cursor only needs even distribution, not cross-request ordering.
    pub fn start(&self) -> usize {
        self.inner.cursor.fetch_add(1, Ordering::Relaxed) % self.inner.keys.len()
    }

    pub fn key(&self, index: usize) -> &str {
        &self.inner.keys[index % self.inner.keys.len()]
    }
}

/// HTTP statuses that indict the key rather than the request or upstream:
/// 401/403 invalid or unauthorized, 429 rate limited, 432/433 provider quota.
pub fn is_key_scoped_status(status: u16) -> bool {
    matches!(status, 401 | 403 | 429 | 432 | 433)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_key_parses_to_one_entry() {
        let pool = KeyPool::parse("only-key");
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.key(0), "only-key");
    }

    #[test]
    fn comma_separated_keys_split_trim_and_drop_empties() {
        let pool = KeyPool::parse(" key-a, key-b ,, key-c,");
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.key(0), "key-a");
        assert_eq!(pool.key(1), "key-b");
        assert_eq!(pool.key(2), "key-c");
    }

    #[test]
    fn all_empty_segments_fall_back_to_raw_value() {
        let pool = KeyPool::parse("");
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.key(0), "");
    }

    #[test]
    fn start_rotates_round_robin_across_requests() {
        let pool = KeyPool::parse("a,b,c");
        assert_eq!(pool.start(), 0);
        assert_eq!(pool.start(), 1);
        assert_eq!(pool.start(), 2);
        assert_eq!(pool.start(), 0);
    }

    #[test]
    fn key_indexing_wraps_for_failover_offsets() {
        let pool = KeyPool::parse("a,b");
        assert_eq!(pool.key(2), "a");
        assert_eq!(pool.key(3), "b");
    }

    #[test]
    fn rotation_cursor_is_shared_across_clones() {
        let pool = KeyPool::parse("a,b");
        let clone = pool.clone();
        assert_eq!(pool.start(), 0);
        assert_eq!(clone.start(), 1);
        assert_eq!(pool.start(), 0);
    }

    #[test]
    fn key_scoped_statuses_trigger_rotation_only() {
        for status in [401, 403, 429, 432, 433] {
            assert!(is_key_scoped_status(status), "expected rotate on {status}");
        }
        for status in [400, 404, 408, 500, 502, 503] {
            assert!(!is_key_scoped_status(status), "must not rotate on {status}");
        }
    }
}
