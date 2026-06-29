use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::Config;
use crate::paths::resolve_config_path;
use crate::schema::ConfigFile;
use crate::template;

#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("read config {} failed: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parse config {} failed: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("debug log path {} is not writable: {source}", path.display())]
    DebugLogPath {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Outcome of a `--init` scaffold attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    Created,
    AlreadyExists,
}

/// Idempotent template writer used by `grok-search-rs --init`. Returns
/// `AlreadyExists` without touching the file when it already exists; otherwise
/// creates parent dirs and writes the annotated template (all keys commented).
pub fn write_template(path: &Path) -> std::io::Result<InitOutcome> {
    if path.exists() {
        return Ok(InitOutcome::AlreadyExists);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, config_template())?;
    Ok(InitOutcome::Created)
}

/// Generated TOML template. All keys are commented so an empty scaffold cannot
/// silently override built-in defaults; the user uncomments only what they need.
pub fn config_template() -> String {
    template::build_config_template()
}

/// Stable snapshot of the generated TOML template for compatibility with code
/// that imports this constant. Tests verify this stays equal to
/// [`config_template`].
pub const CONFIG_TEMPLATE: &str = include_str!("generated/config-template.toml");

pub(crate) fn try_load_from_env_map(
    env_map: HashMap<String, String>,
) -> std::result::Result<Config, ConfigLoadError> {
    let file_map = resolve_config_path(&env_map)
        .map(|path| load_file_map(&path))
        .transpose()?
        .flatten()
        .unwrap_or_default();
    let mut config = Config::from_env_map(merge_env_over_file(file_map, env_map));
    config.apply_github_cli_token_fallback();
    validate_debug_log_path(&config)?;
    Ok(config)
}

fn load_file_map(
    path: &Path,
) -> std::result::Result<Option<HashMap<String, String>>, ConfigLoadError> {
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file = toml::from_str::<ConfigFile>(&body).map_err(|source| ConfigLoadError::Parse {
        path: path.to_path_buf(),
        source: Box::new(source),
    })?;
    Ok(Some(file.into_env_map()))
}

fn merge_env_over_file(
    mut base: HashMap<String, String>,
    overlay: HashMap<String, String>,
) -> HashMap<String, String> {
    for (k, v) in overlay {
        base.insert(k, v);
    }
    base
}

fn validate_debug_log_path(config: &Config) -> std::result::Result<(), ConfigLoadError> {
    let Some(path) = config.debug_log_path.as_ref() else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|source| ConfigLoadError::DebugLogPath {
            path: path.clone(),
            source,
        })?;
    }
    let _file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| ConfigLoadError::DebugLogPath {
            path: path.clone(),
            source,
        })?;
    Ok(())
}
