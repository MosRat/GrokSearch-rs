use std::collections::HashMap;
use std::path::PathBuf;

use crate::schema::{ConfigKey, RedactionKind};
use crate::util::{bool_literal, normalize_plain_base, normalize_v1_base};

pub(crate) struct ConfigReader<'a> {
    map: &'a HashMap<String, String>,
}

impl<'a> ConfigReader<'a> {
    pub(crate) fn new(map: &'a HashMap<String, String>) -> Self {
        Self { map }
    }

    pub(crate) fn optional(&self, key: ConfigKey) -> Option<String> {
        key.env_aliases
            .iter()
            .find_map(|env| self.optional_env(env))
    }

    pub(crate) fn secret(&self, key: ConfigKey) -> Option<String> {
        debug_assert_eq!(key.redaction, RedactionKind::SecretStatus);
        self.optional(key)
    }

    pub(crate) fn string(&self, key: ConfigKey, default: &str) -> String {
        self.optional(key).unwrap_or_else(|| default.to_string())
    }

    pub(crate) fn bool(&self, key: ConfigKey, default: bool) -> bool {
        self.optional(key)
            .map(|value| bool_literal(&value))
            .unwrap_or(default)
    }

    pub(crate) fn usize(&self, key: ConfigKey, default: usize) -> usize {
        self.optional(key)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default)
    }

    pub(crate) fn u64(&self, key: ConfigKey, default: u64) -> u64 {
        self.optional(key)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default)
    }

    pub(crate) fn positive_usize(&self, key: ConfigKey) -> Option<usize> {
        self.optional(key)
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
    }

    pub(crate) fn plain_base_url(&self, key: ConfigKey, default: &str) -> String {
        normalize_plain_base(&self.string(key, default))
    }

    pub(crate) fn v1_base_url(&self, key: ConfigKey, default: &str) -> String {
        normalize_v1_base(&self.string(key, default))
    }

    pub(crate) fn path(&self, key: ConfigKey) -> Option<PathBuf> {
        self.optional(key).map(PathBuf::from)
    }

    fn optional_env(&self, env: &str) -> Option<String> {
        self.map
            .get(env)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }
}
