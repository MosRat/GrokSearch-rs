use std::fs::{self, OpenOptions};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) async fn wait_for_global_provider_rate_limit(provider: &str, min_interval: Duration) {
    let base = std::env::temp_dir();
    let stamp_path = base.join(format!("grok-search-rs-{provider}.timestamp"));
    let lock_path = base.join(format!("grok-search-rs-{provider}.lock"));

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_lock) => {
                let _guard = ProviderRateLimitLock { path: lock_path };
                if let Ok(last) = fs::read_to_string(&stamp_path)
                    .ok()
                    .and_then(|raw| raw.trim().parse::<u128>().ok())
                    .ok_or(())
                {
                    let now = unix_millis();
                    let elapsed = now.saturating_sub(last);
                    let min = min_interval.as_millis();
                    if elapsed < min {
                        tokio::time::sleep(Duration::from_millis((min - elapsed) as u64)).await;
                    }
                }
                let _ = fs::write(stamp_path, unix_millis().to_string());
                return;
            }
            Err(_) => {
                if fs::metadata(&lock_path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(10))
                {
                    let _ = fs::remove_file(&lock_path);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

pub(super) fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

struct ProviderRateLimitLock {
    path: std::path::PathBuf,
}

impl Drop for ProviderRateLimitLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
