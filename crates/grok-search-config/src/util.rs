use crate::schema::{ConfigItem, ConfigKey, CONFIG_ITEMS};

pub(crate) fn secret_status<T>(value: &Option<T>) -> &'static str {
    if value.is_some() {
        "set"
    } else {
        "unset"
    }
}

pub fn normalize_v1_base(url: &str) -> String {
    let mut value = url.trim().trim_end_matches('/').to_string();
    // Strip any known full-endpoint suffix so callers can pass either a base
    // URL or a full endpoint and converge on the same `/v1` form.
    for suffix in ["/chat/completions", "/responses"] {
        if value.ends_with(suffix) {
            let keep = value.len() - suffix.len();
            value.truncate(keep);
            value = value.trim_end_matches('/').to_string();
        }
    }
    if !value.ends_with("/v1") {
        value.push_str("/v1");
    }
    value
}

pub(crate) fn normalize_plain_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

pub(crate) fn bool_literal(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

pub(crate) fn redact_proxy_url_for_config(raw: &str) -> String {
    if raw.eq_ignore_ascii_case("auto") || raw.eq_ignore_ascii_case("off") {
        return raw.to_string();
    }
    let Ok(mut url) = url::Url::parse(raw) else {
        return "<invalid proxy url>".to_string();
    };
    if !url.username().is_empty() {
        let _ = url.set_username("***");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

pub(crate) fn redact_optional_url(raw: &Option<String>) -> Option<String> {
    let raw = raw.as_deref()?;
    Some(redact_url(raw))
}

pub(crate) fn redact_url(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return "<invalid url>".to_string();
    };
    if !url.username().is_empty() {
        let _ = url.set_username("***");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn schema_item(key: ConfigKey) -> &'static ConfigItem {
    CONFIG_ITEMS
        .iter()
        .find(|item| item.key == key)
        .unwrap_or_else(|| panic!("missing schema item for {}", key.toml_key))
}

pub(crate) fn default_str(key: ConfigKey) -> &'static str {
    let default = schema_item(key).default_display;
    debug_assert_ne!(default, "unset");
    default
}

pub(crate) fn default_bool(key: ConfigKey) -> bool {
    bool_literal(schema_item(key).default_display)
}

pub(crate) fn default_usize(key: ConfigKey) -> usize {
    schema_item(key)
        .default_display
        .parse()
        .unwrap_or_else(|_| panic!("{} default must be usize", key.toml_key))
}

pub(crate) fn default_u64(key: ConfigKey) -> u64 {
    schema_item(key)
        .default_display
        .parse()
        .unwrap_or_else(|_| panic!("{} default must be u64", key.toml_key))
}
