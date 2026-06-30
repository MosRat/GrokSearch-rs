use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn resolve_config_path(env: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(explicit) = env.get("GROK_SEARCH_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let home = resolve_home_dir(env)?;
    Some(
        home.join(".config")
            .join("grok-search-rs")
            .join("config.toml"),
    )
}

/// Cross-platform home directory resolution. Reads `$HOME` first (Unix and
/// Git Bash / MSYS on Windows both set it), then falls back to
/// `%USERPROFILE%` for native Windows shells (PowerShell, cmd) where `HOME`
/// is not part of the default environment. Env-driven so tests can inject
/// either layout without touching real process env.
fn resolve_home_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    if let Some(home) = env.get("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = env.get("USERPROFILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    None
}

/// Resolved config file path using process env. Precedence:
/// 1. `$GROK_SEARCH_CONFIG` (any platform, explicit override)
/// 2. `$HOME/.config/grok-search-rs/config.toml` (Unix / Git Bash)
/// 3. `%USERPROFILE%\.config\grok-search-rs\config.toml` (native Windows)
///
/// Returns `None` only when none of the above are set.
pub fn config_path() -> Option<PathBuf> {
    let env: HashMap<String, String> = std::env::vars().collect();
    resolve_config_path(&env)
}

pub fn auth_path() -> Option<PathBuf> {
    auth_path_for(std::env::vars())
}

pub fn progressive_cache_path() -> Option<PathBuf> {
    progressive_cache_path_for(std::env::vars())
}

pub fn auth_path_for<I, K, V>(env_vars: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let env: HashMap<String, String> = env_vars
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    if let Some(explicit) = env.get("GROK_SEARCH_AUTH_FILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    resolve_config_path(&env).map(|path| path.with_file_name("auth.json"))
}

pub fn progressive_cache_path_for<I, K, V>(env_vars: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let env: HashMap<String, String> = env_vars
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    if let Some(explicit) = env
        .get("GROK_SEARCH_PROGRESSIVE_CACHE_PATH")
        .filter(|v| !v.is_empty())
    {
        return Some(PathBuf::from(explicit));
    }
    resolve_config_path(&env).map(|path| path.with_file_name("progressive-cache.redb"))
}

/// Test-friendly variant of [`config_path`] that takes an explicit env map.
/// Lets integration tests assert path resolution across platforms without
/// mutating process-global env state.
pub fn config_path_for<I, K, V>(env_vars: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let env: HashMap<String, String> = env_vars
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    resolve_config_path(&env)
}
