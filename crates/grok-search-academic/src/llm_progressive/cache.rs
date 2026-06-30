use std::path::PathBuf;

use grok_search_cache::{ProgressiveCachePut, ProgressiveCacheStore, RedbProgressiveCache};
use grok_search_types::Result;

use super::ProgressiveRunConfig;

#[derive(Debug, Clone)]
pub(crate) struct ProgressiveCachePlan {
    pub path: PathBuf,
    pub key: String,
    pub ttl_seconds: Option<u64>,
    pub max_entries: usize,
    pub pdf_sha256: String,
    pub input_text_sha256: String,
    pub strategy_hash: String,
    pub model: String,
    pub input_profile: String,
    pub prompt_profile: String,
}

pub(crate) async fn cache_get(
    path: PathBuf,
    key: String,
) -> Result<Option<grok_search_cache::ProgressiveCacheEntry>> {
    tokio::task::spawn_blocking(move || {
        let cache = RedbProgressiveCache::open(path)?;
        cache.get(&key)
    })
    .await
    .map_err(|err| grok_search_types::GrokSearchError::Io(format!("cache task failed: {err}")))?
}

pub(crate) async fn cache_put(plan: ProgressiveCachePlan, bytes: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let cache = RedbProgressiveCache::open(&plan.path)?;
        cache.put(
            ProgressiveCachePut {
                cache_key: plan.key,
                bytes,
                ttl_seconds: plan.ttl_seconds,
                pdf_sha256: plan.pdf_sha256,
                input_text_sha256: plan.input_text_sha256,
                strategy_hash: plan.strategy_hash,
                model: plan.model,
                input_profile: plan.input_profile,
                prompt_profile: plan.prompt_profile,
            },
            plan.max_entries,
        )?;
        Ok(())
    })
    .await
    .map_err(|err| grok_search_types::GrokSearchError::Io(format!("cache task failed: {err}")))?
}

pub(crate) fn strategy_hash(
    config: &ProgressiveRunConfig,
    pdf_sha256: &str,
    input_text_sha256: &str,
) -> String {
    let payload = format!(
        "schema=v1\nprompt=v2\npdf={pdf_sha256}\ntext={input_text_sha256}\nmodel={}\ninput={}\nprompt={}\nchunk=paragraph_window\nmax_chunk={}\noverlap={}\nmax_output={}",
        config.model,
        config.input_profile,
        config.prompt_profile,
        config.max_chunk_chars,
        config.overlap_chars,
        config.max_output_tokens
    );
    sha256_hex(payload.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
