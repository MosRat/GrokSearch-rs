use crate::schema::{ConfigKey, RedactionKind};
use crate::util::{redact_proxy_url_for_config, redact_url, secret_status};

pub(crate) struct DiagnosticField {
    key: ConfigKey,
    value: String,
}

impl DiagnosticField {
    pub(crate) fn render(self) -> String {
        format!("{}={}", self.key.toml_key, self.value)
    }
}

pub(crate) fn diagnostic_pair(key: ConfigKey, raw: Option<String>) -> DiagnosticField {
    let value = match key.redaction {
        RedactionKind::None | RedactionKind::Path => raw.unwrap_or_else(|| "unset".to_string()),
        RedactionKind::SecretStatus => secret_status(&raw).to_string(),
        RedactionKind::Url => raw
            .as_deref()
            .map(redact_url)
            .unwrap_or_else(|| "unset".to_string()),
        RedactionKind::ProxyUrl => raw
            .as_deref()
            .map(redact_proxy_url_for_config)
            .unwrap_or_else(|| "unset".to_string()),
    };
    DiagnosticField { key, value }
}

pub(crate) trait DebugRedacted {
    fn fmt_debug_redacted(&self, key: ConfigKey) -> String;
}

impl DebugRedacted for Option<String> {
    fn fmt_debug_redacted(&self, key: ConfigKey) -> String {
        match key.redaction {
            RedactionKind::SecretStatus => secret_status(self).to_string(),
            RedactionKind::Url => self
                .as_deref()
                .map(redact_url)
                .unwrap_or_else(|| "unset".to_string()),
            RedactionKind::ProxyUrl => self
                .as_deref()
                .map(redact_proxy_url_for_config)
                .unwrap_or_else(|| "unset".to_string()),
            RedactionKind::None | RedactionKind::Path => self
                .as_deref()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| "unset".to_string()),
        }
    }
}

impl DebugRedacted for String {
    fn fmt_debug_redacted(&self, key: ConfigKey) -> String {
        match key.redaction {
            RedactionKind::Url => redact_url(self),
            RedactionKind::ProxyUrl => redact_proxy_url_for_config(self),
            RedactionKind::SecretStatus => "set".to_string(),
            RedactionKind::None | RedactionKind::Path => self.clone(),
        }
    }
}
