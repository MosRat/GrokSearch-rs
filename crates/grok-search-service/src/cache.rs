use std::num::NonZeroUsize;
use std::sync::Arc;

use grok_search_types::model::source::Source;
use lru::LruCache;

#[derive(Debug, Clone)]
pub struct SourceCache {
    values: LruCache<String, Arc<Vec<Source>>>,
}

impl SourceCache {
    pub fn new(max_size: usize) -> Self {
        let capacity = NonZeroUsize::new(max_size.max(1)).expect("max(1) is non-zero");
        Self {
            values: LruCache::new(capacity),
        }
    }

    pub fn set(&mut self, session_id: String, sources: Arc<Vec<Source>>) {
        self.values.put(session_id, sources);
    }

    pub fn get(&mut self, session_id: &str) -> Option<Arc<Vec<Source>>> {
        self.values.get(session_id).cloned()
    }
}
