use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::diagnostics::{diagnostic_pair, DebugRedacted, DiagnosticField};
use crate::loader::{try_load_from_env_map, ConfigLoadError};
use crate::reader::ConfigReader;
use crate::schema::*;
use crate::util::{
    default_bool, default_str, default_u64, default_usize, redact_optional_url, secret_status,
};
use crate::MAX_INLINE_SOURCES_LIMIT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    ApiKey,
    OAuth,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub grok_api_url: String,
    pub grok_api_key: Option<String>,
    pub grok_auth_mode: AuthMode,
    pub grok_auth_file: Option<PathBuf>,
    pub grok_model: String,
    pub web_search_enabled: bool,
    pub x_search_enabled: bool,
    pub tavily_api_url: String,
    pub tavily_api_key: Option<String>,
    pub tavily_enabled: bool,
    pub firecrawl_api_url: String,
    pub firecrawl_api_key: Option<String>,
    pub firecrawl_enabled: bool,
    pub default_extra_sources: usize,
    pub fallback_sources: usize,
    pub fetch_max_chars: Option<usize>,
    pub cache_size: usize,
    pub timeout: Duration,
    pub proxy: String,
    pub openai_compatible_api_url: Option<String>,
    pub openai_compatible_api_key: Option<String>,
    pub openai_compatible_model: Option<String>,
    pub transport: Transport,
    pub github_token: Option<String>,
    pub source_max_answers: usize,
    pub source_max_comments: usize,
    pub enrich_concurrency: usize,
    pub enrich_max_chars: usize,
    pub max_inline_sources: usize,
    pub response_max_chars: usize,
    pub max_response_bytes: usize,
    pub debug_log_path: Option<PathBuf>,
    pub academic_enabled: bool,
    pub academic_email: Option<String>,
    pub semantic_scholar_api_key: Option<String>,
    pub openalex_api_key: Option<String>,
    pub zhihu_api_key: Option<String>,
    pub zhihu_openapi_base_url: String,
    pub zhihu_search_url: Option<String>,
    pub academic_scihub_enabled: bool,
    pub academic_scihub_base_url: Option<String>,
    pub academic_institutional_enabled: bool,
    pub academic_institutional_accept_invalid_certs: bool,
    pub academic_institutional_probe: bool,
    pub academic_max_pdf_bytes: usize,
    pub academic_pdf_max_chars: Option<usize>,
}

/// Hand-written `Debug` that masks secret-bearing fields so a stray
/// `{:?}`/`{:#?}` of a `Config` can never leak credentials. Secret `Option`
/// fields render as a two-state `"set"`/`"unset"` marker (mirroring
/// [`Config::github_token_status`]); every non-secret field stays readable.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field(
                "grok_api_url",
                &self.grok_api_url.fmt_debug_redacted(GROK_API_URL),
            )
            .field(
                "grok_api_key",
                &self.grok_api_key.fmt_debug_redacted(GROK_API_KEY),
            )
            .field("grok_auth_mode", &self.grok_auth_mode)
            .field("grok_auth_file", &self.grok_auth_file)
            .field("grok_model", &self.grok_model)
            .field("web_search_enabled", &self.web_search_enabled)
            .field("x_search_enabled", &self.x_search_enabled)
            .field("tavily_api_url", &self.tavily_api_url)
            .field(
                "tavily_api_key",
                &self.tavily_api_key.fmt_debug_redacted(TAVILY_API_KEY),
            )
            .field("tavily_enabled", &self.tavily_enabled)
            .field("firecrawl_api_url", &self.firecrawl_api_url)
            .field(
                "firecrawl_api_key",
                &self.firecrawl_api_key.fmt_debug_redacted(FIRECRAWL_API_KEY),
            )
            .field("firecrawl_enabled", &self.firecrawl_enabled)
            .field("default_extra_sources", &self.default_extra_sources)
            .field("fallback_sources", &self.fallback_sources)
            .field("fetch_max_chars", &self.fetch_max_chars)
            .field("cache_size", &self.cache_size)
            .field("timeout", &self.timeout)
            .field("proxy", &self.proxy.fmt_debug_redacted(PROXY))
            .field("openai_compatible_api_url", &self.openai_compatible_api_url)
            .field(
                "openai_compatible_api_key",
                &self
                    .openai_compatible_api_key
                    .fmt_debug_redacted(OPENAI_COMPATIBLE_API_KEY),
            )
            .field("openai_compatible_model", &self.openai_compatible_model)
            .field("transport", &self.transport)
            .field(
                "github_token",
                &self.github_token.fmt_debug_redacted(GITHUB_TOKEN),
            )
            .field("source_max_answers", &self.source_max_answers)
            .field("source_max_comments", &self.source_max_comments)
            .field("enrich_concurrency", &self.enrich_concurrency)
            .field("enrich_max_chars", &self.enrich_max_chars)
            .field("max_inline_sources", &self.max_inline_sources)
            .field("response_max_chars", &self.response_max_chars)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("debug_log_path", &self.debug_log_path)
            .field("academic_enabled", &self.academic_enabled)
            .field("academic_email", &self.academic_email_status())
            .field(
                "semantic_scholar_api_key",
                &self
                    .semantic_scholar_api_key
                    .fmt_debug_redacted(SEMANTIC_SCHOLAR_API_KEY),
            )
            .field(
                "openalex_api_key",
                &self.openalex_api_key.fmt_debug_redacted(OPENALEX_API_KEY),
            )
            .field(
                "zhihu_api_key",
                &self.zhihu_api_key.fmt_debug_redacted(ZHIHU_API_KEY),
            )
            .field(
                "zhihu_openapi_base_url",
                &self
                    .zhihu_openapi_base_url
                    .fmt_debug_redacted(ZHIHU_OPENAPI_BASE_URL),
            )
            .field(
                "zhihu_search_url",
                &self.zhihu_search_url.fmt_debug_redacted(ZHIHU_SEARCH_URL),
            )
            .field("academic_scihub_enabled", &self.academic_scihub_enabled)
            .field(
                "academic_scihub_base_url",
                &self
                    .academic_scihub_base_url
                    .fmt_debug_redacted(ACADEMIC_SCIHUB_BASE_URL),
            )
            .field(
                "academic_institutional_enabled",
                &self.academic_institutional_enabled,
            )
            .field(
                "academic_institutional_accept_invalid_certs",
                &self.academic_institutional_accept_invalid_certs,
            )
            .field(
                "academic_institutional_probe",
                &self.academic_institutional_probe,
            )
            .field("academic_max_pdf_bytes", &self.academic_max_pdf_bytes)
            .field("academic_pdf_max_chars", &self.academic_pdf_max_chars)
            .finish()
    }
}

impl Config {
    /// Load config with full precedence chain: process env > config file > defaults.
    /// Config file path: `$GROK_SEARCH_CONFIG` if set, else
    /// `<home>/.config/grok-search-rs/config.toml`, where `<home>` is `$HOME`
    /// on Unix / Git Bash and `%USERPROFILE%` on native Windows shells.
    /// Missing or unparseable files are skipped silently (env-only mode).
    pub fn load() -> Self {
        Self::load_from(std::env::vars())
    }

    pub fn try_load() -> std::result::Result<Self, ConfigLoadError> {
        Self::try_load_from(std::env::vars())
    }

    /// Same as `load`, but uses a caller-supplied env map. Lets tests exercise
    /// the file + env merge without mutating process-global env state.
    pub fn load_from<I, K, V>(env_vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let env_vec: Vec<(String, String)> = env_vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        Self::try_load_from(env_vec.clone()).unwrap_or_else(|err| {
            eprintln!("grok-search-rs: {err}; falling back to env/defaults");
            Self::from_env_map(env_vec)
        })
    }

    pub fn try_load_from<I, K, V>(env_vars: I) -> std::result::Result<Self, ConfigLoadError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let env_map: HashMap<String, String> = env_vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        try_load_from_env_map(env_map)
    }

    pub fn from_env() -> Self {
        let mut config = Self::from_env_map(std::env::vars());
        config.apply_github_cli_token_fallback();
        config
    }

    pub fn from_env_map<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let map: HashMap<String, String> = vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let reader = ConfigReader::new(&map);
        let grok_auth_mode = auth_mode_value(&reader);

        Self {
            grok_api_url: reader.v1_base_url(GROK_API_URL, default_str(GROK_API_URL)),
            grok_api_key: reader.secret(GROK_API_KEY),
            grok_auth_mode,
            grok_auth_file: reader.path(GROK_AUTH_FILE),
            grok_model: reader.string(GROK_MODEL, default_str(GROK_MODEL)),
            web_search_enabled: reader.bool(WEB_SEARCH_ENABLED, default_bool(WEB_SEARCH_ENABLED)),
            x_search_enabled: reader.bool(X_SEARCH_ENABLED, default_bool(X_SEARCH_ENABLED)),
            tavily_api_url: reader.plain_base_url(TAVILY_API_URL, default_str(TAVILY_API_URL)),
            tavily_api_key: reader.secret(TAVILY_API_KEY),
            tavily_enabled: reader.bool(TAVILY_ENABLED, default_bool(TAVILY_ENABLED)),
            firecrawl_api_url: reader
                .v1_base_url(FIRECRAWL_API_URL, default_str(FIRECRAWL_API_URL)),
            firecrawl_api_key: reader.secret(FIRECRAWL_API_KEY),
            firecrawl_enabled: reader.bool(FIRECRAWL_ENABLED, default_bool(FIRECRAWL_ENABLED)),
            default_extra_sources: reader
                .usize(DEFAULT_EXTRA_SOURCES, default_usize(DEFAULT_EXTRA_SOURCES)),
            fallback_sources: reader.usize(FALLBACK_SOURCES, default_usize(FALLBACK_SOURCES)),
            fetch_max_chars: reader.positive_usize(FETCH_MAX_CHARS),
            cache_size: reader.usize(CACHE_SIZE, default_usize(CACHE_SIZE)),
            timeout: Duration::from_secs(reader.u64(TIMEOUT_SECONDS, default_u64(TIMEOUT_SECONDS))),
            proxy: reader.string(PROXY, default_str(PROXY)),
            openai_compatible_api_url: reader.optional(OPENAI_COMPATIBLE_API_URL),
            openai_compatible_api_key: reader.secret(OPENAI_COMPATIBLE_API_KEY),
            openai_compatible_model: reader.optional(OPENAI_COMPATIBLE_MODEL),
            transport: decide_transport(&reader, grok_auth_mode),
            github_token: reader.secret(GITHUB_TOKEN),
            source_max_answers: reader.usize(SOURCE_MAX_ANSWERS, default_usize(SOURCE_MAX_ANSWERS)),
            source_max_comments: reader
                .usize(SOURCE_MAX_COMMENTS, default_usize(SOURCE_MAX_COMMENTS)),
            enrich_concurrency: reader
                .usize(ENRICH_CONCURRENCY, default_usize(ENRICH_CONCURRENCY))
                .clamp(1, 5),
            enrich_max_chars: reader.usize(ENRICH_MAX_CHARS, default_usize(ENRICH_MAX_CHARS)),
            max_inline_sources: reader
                .usize(MAX_INLINE_SOURCES, default_usize(MAX_INLINE_SOURCES))
                .min(MAX_INLINE_SOURCES_LIMIT),
            response_max_chars: reader.usize(RESPONSE_MAX_CHARS, default_usize(RESPONSE_MAX_CHARS)),
            max_response_bytes: reader.usize(MAX_RESPONSE_BYTES, default_usize(MAX_RESPONSE_BYTES)),
            debug_log_path: reader.path(DEBUG_LOG_PATH),
            academic_enabled: reader.bool(ACADEMIC_ENABLED, default_bool(ACADEMIC_ENABLED)),
            academic_email: reader.optional(ACADEMIC_EMAIL),
            semantic_scholar_api_key: reader.secret(SEMANTIC_SCHOLAR_API_KEY),
            openalex_api_key: reader.secret(OPENALEX_API_KEY),
            zhihu_api_key: reader.secret(ZHIHU_API_KEY),
            zhihu_openapi_base_url: reader
                .plain_base_url(ZHIHU_OPENAPI_BASE_URL, default_str(ZHIHU_OPENAPI_BASE_URL)),
            zhihu_search_url: reader.optional(ZHIHU_SEARCH_URL),
            academic_scihub_enabled: reader.bool(
                ACADEMIC_SCIHUB_ENABLED,
                default_bool(ACADEMIC_SCIHUB_ENABLED),
            ),
            academic_scihub_base_url: reader.optional(ACADEMIC_SCIHUB_BASE_URL),
            academic_institutional_enabled: reader.bool(
                ACADEMIC_INSTITUTIONAL_ENABLED,
                default_bool(ACADEMIC_INSTITUTIONAL_ENABLED),
            ),
            academic_institutional_accept_invalid_certs: reader.bool(
                ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS,
                default_bool(ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS),
            ),
            academic_institutional_probe: reader.bool(
                ACADEMIC_INSTITUTIONAL_PROBE,
                default_bool(ACADEMIC_INSTITUTIONAL_PROBE),
            ),
            academic_max_pdf_bytes: reader.usize(
                ACADEMIC_MAX_PDF_BYTES,
                default_usize(ACADEMIC_MAX_PDF_BYTES),
            ),
            academic_pdf_max_chars: reader.positive_usize(ACADEMIC_PDF_MAX_CHARS),
        }
    }

    /// Two-state presence signal for GITHUB_TOKEN. Reports only whether a
    /// token is configured - never the value or any fragment.
    pub fn github_token_status(&self) -> &'static str {
        secret_status(&self.github_token)
    }

    pub(crate) fn apply_github_cli_token_fallback(&mut self) {
        if self.github_token.is_some() {
            return;
        }
        self.github_token = github_token_from_gh_cli();
    }

    pub fn academic_email_status(&self) -> &'static str {
        secret_status(&self.academic_email)
    }

    pub fn semantic_scholar_key_status(&self) -> &'static str {
        secret_status(&self.semantic_scholar_api_key)
    }

    pub fn openalex_key_status(&self) -> &'static str {
        secret_status(&self.openalex_api_key)
    }

    pub fn zhihu_key_status(&self) -> &'static str {
        secret_status(&self.zhihu_api_key)
    }

    pub fn redacted_scihub_base_url(&self) -> Option<String> {
        redact_optional_url(&self.academic_scihub_base_url)
    }

    pub fn redacted_diagnostics(&self) -> String {
        let fields = [
            diagnostic_pair(GROK_API_URL, Some(self.grok_api_url.clone())),
            diagnostic_pair(GROK_API_KEY, self.grok_api_key.clone()),
            diagnostic_pair(GROK_AUTH_MODE, Some(format!("{:?}", self.grok_auth_mode))),
            diagnostic_pair(
                GROK_AUTH_FILE,
                Some(
                    self.grok_auth_file
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "default".to_string()),
                ),
            ),
            diagnostic_pair(GROK_MODEL, Some(self.grok_model.clone())),
            diagnostic_pair(
                WEB_SEARCH_ENABLED,
                Some(self.web_search_enabled.to_string()),
            ),
            diagnostic_pair(X_SEARCH_ENABLED, Some(self.x_search_enabled.to_string())),
            diagnostic_pair(TAVILY_API_KEY, self.tavily_api_key.clone()),
            diagnostic_pair(FIRECRAWL_API_KEY, self.firecrawl_api_key.clone()),
            diagnostic_pair(
                DEFAULT_EXTRA_SOURCES,
                Some(self.default_extra_sources.to_string()),
            ),
            diagnostic_pair(FALLBACK_SOURCES, Some(self.fallback_sources.to_string())),
            diagnostic_pair(TIMEOUT_SECONDS, Some(self.timeout.as_secs().to_string())),
            diagnostic_pair(PROXY, Some(self.proxy.clone())),
            diagnostic_pair(GITHUB_TOKEN, self.github_token.clone()),
            diagnostic_pair(
                MAX_RESPONSE_BYTES,
                Some(self.max_response_bytes.to_string()),
            ),
            diagnostic_pair(
                DEBUG_LOG_PATH,
                Some(
                    self.debug_log_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "unset".to_string()),
                ),
            ),
            diagnostic_pair(ACADEMIC_ENABLED, Some(self.academic_enabled.to_string())),
            diagnostic_pair(ACADEMIC_EMAIL, self.academic_email.clone()),
            diagnostic_pair(
                SEMANTIC_SCHOLAR_API_KEY,
                self.semantic_scholar_api_key.clone(),
            ),
            diagnostic_pair(OPENALEX_API_KEY, self.openalex_api_key.clone()),
            diagnostic_pair(ZHIHU_API_KEY, self.zhihu_api_key.clone()),
            diagnostic_pair(
                ZHIHU_OPENAPI_BASE_URL,
                Some(self.zhihu_openapi_base_url.clone()),
            ),
            diagnostic_pair(ZHIHU_SEARCH_URL, self.zhihu_search_url.clone()),
            diagnostic_pair(
                ACADEMIC_SCIHUB_ENABLED,
                Some(self.academic_scihub_enabled.to_string()),
            ),
            diagnostic_pair(
                ACADEMIC_SCIHUB_BASE_URL,
                self.academic_scihub_base_url.clone(),
            ),
            diagnostic_pair(
                ACADEMIC_INSTITUTIONAL_ENABLED,
                Some(self.academic_institutional_enabled.to_string()),
            ),
            diagnostic_pair(
                ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS,
                Some(self.academic_institutional_accept_invalid_certs.to_string()),
            ),
            diagnostic_pair(
                ACADEMIC_INSTITUTIONAL_PROBE,
                Some(self.academic_institutional_probe.to_string()),
            ),
        ];
        fields
            .into_iter()
            .map(DiagnosticField::render)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn github_token_from_gh_cli() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    github_token_from_gh_stdout(&output.stdout)
}

#[cfg(test)]
fn github_token_from_gh_stdout(stdout: &[u8]) -> Option<String> {
    github_token_from_stdout(stdout)
}

#[cfg(not(test))]
fn github_token_from_gh_stdout(stdout: &[u8]) -> Option<String> {
    github_token_from_stdout(stdout)
}

fn github_token_from_stdout(stdout: &[u8]) -> Option<String> {
    let token = String::from_utf8_lossy(stdout).trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn auth_mode_value(reader: &ConfigReader<'_>) -> AuthMode {
    match reader
        .optional(GROK_AUTH_MODE)
        .map(|value| (value.trim().to_string(), value.trim().to_ascii_lowercase()))
    {
        Some((_, value)) if value == "api_key" || value.is_empty() => AuthMode::ApiKey,
        Some((_, value)) if value == "oauth" => AuthMode::OAuth,
        Some((raw, _)) => {
            eprintln!(
                "unknown GROK_SEARCH_AUTH_MODE=\"{}\"; falling back to api_key. Valid values: api_key, oauth.",
                raw
            );
            AuthMode::ApiKey
        }
        _ => AuthMode::ApiKey,
    }
}

fn decide_transport(reader: &ConfigReader<'_>, auth_mode: AuthMode) -> Transport {
    if auth_mode == AuthMode::OAuth {
        return Transport::Responses;
    }
    let grok_key_set = reader.secret(GROK_API_KEY).is_some();
    let compat_url_set = reader.optional(OPENAI_COMPATIBLE_API_URL).is_some();
    let compat_key_set = reader.secret(OPENAI_COMPATIBLE_API_KEY).is_some();

    if grok_key_set {
        return Transport::Responses;
    }
    if compat_url_set && compat_key_set {
        return Transport::ChatCompletions;
    }
    Transport::Responses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_cli_token_stdout_is_trimmed_and_filtered() {
        assert_eq!(
            github_token_from_gh_stdout(b"gho_from_cli\r\n").as_deref(),
            Some("gho_from_cli")
        );
        assert_eq!(github_token_from_gh_stdout(b" \n\t "), None);
    }

    #[test]
    fn transport_defaults_to_responses_when_only_grok_set() {
        let cfg = Config::from_env_map([("GROK_SEARCH_API_KEY", "xai-fake")]);
        assert_eq!(cfg.transport, Transport::Responses);
    }

    #[test]
    fn transport_chat_completions_when_only_compat_set() {
        let cfg = Config::from_env_map([
            ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
        ]);
        assert_eq!(cfg.transport, Transport::ChatCompletions);
    }

    #[test]
    fn transport_prefers_grok_when_both_set() {
        let cfg = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
            ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
        ]);
        assert_eq!(cfg.transport, Transport::Responses);
    }
}
