use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigKey {
    pub(crate) toml_key: &'static str,
    pub(crate) canonical_env: &'static str,
    pub(crate) env_aliases: &'static [&'static str],
    pub(crate) kind: ConfigValueKind,
    pub(crate) redaction: RedactionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigValueKind {
    String,
    Bool,
    Usize,
    DurationSeconds,
    Path,
    UrlBase,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedactionKind {
    None,
    SecretStatus,
    Url,
    ProxyUrl,
    Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigItem {
    pub(crate) field: &'static str,
    pub(crate) group: &'static str,
    pub(crate) rust_type: &'static str,
    pub(crate) key: ConfigKey,
    pub(crate) default_display: &'static str,
    pub(crate) sample_value: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) template: TemplateVisibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateVisibility {
    Commented,
}

trait ConfigFileValue {
    fn into_env_value(self) -> String;
}

impl ConfigFileValue for String {
    fn into_env_value(self) -> String {
        self
    }
}

impl ConfigFileValue for bool {
    fn into_env_value(self) -> String {
        self.to_string()
    }
}

impl ConfigFileValue for usize {
    fn into_env_value(self) -> String {
        self.to_string()
    }
}

impl ConfigFileValue for u64 {
    fn into_env_value(self) -> String {
        self.to_string()
    }
}

fn insert_config_file_value<T: ConfigFileValue>(
    out: &mut HashMap<String, String>,
    key: ConfigKey,
    value: Option<T>,
) {
    if let Some(value) = value {
        out.insert(key.canonical_env.to_string(), value.into_env_value());
    }
}

macro_rules! config_schema {
    (
        $(
            group $group:literal {
                $(
                    $(#[doc = $doc:literal])+
                    $const_name:ident {
                        field: $field:ident,
                        type: $ty:ty,
                        toml: $toml:literal,
                        env: $env:literal,
                        aliases: [$($alias:literal),+ $(,)?],
                        kind: $kind:ident,
                        redaction: $redaction:ident,
                        default: $default:literal,
                        sample: $sample:literal,
                    }
                )+
            }
        )+
    ) => {
        $(
            $(
                $(#[doc = $doc])+
                pub(crate) const $const_name: ConfigKey = ConfigKey {
                    toml_key: $toml,
                    canonical_env: $env,
                    env_aliases: &[$($alias),+],
                    kind: ConfigValueKind::$kind,
                    redaction: RedactionKind::$redaction,
                };
            )+
        )+

        pub(crate) const CONFIG_ITEMS: &[ConfigItem] = &[
            $(
                $(
                    ConfigItem {
                        field: stringify!($field),
                        group: $group,
                        rust_type: stringify!($ty),
                        key: $const_name,
                        default_display: $default,
                        sample_value: $sample,
                        doc: concat!($($doc, "\n"),+),
                        template: TemplateVisibility::Commented,
                    },
                )+
            )+
        ];

        #[cfg(test)]
        pub(crate) const CONFIG_KEYS: &[ConfigKey] = &[
            $(
                $(
                    $const_name,
                )+
            )+
        ];

        /// Mirror of `Config` for TOML deserialization. All fields optional so users
        /// only need to set what they care about. Field names map 1:1 to TOML keys.
        #[derive(Debug, Default, Deserialize)]
        #[serde(deny_unknown_fields, default)]
        pub(crate) struct ConfigFile {
            $(
                $(
                    $(#[doc = $doc])+
                    pub(crate) $field: Option<$ty>,
                )+
            )+
        }

        impl ConfigFile {
            /// Translate file fields into the env-style key/value map the rest of the
            /// loader consumes. Keeps a single precedence pipeline.
            pub(crate) fn into_env_map(self) -> HashMap<String, String> {
                let mut out = HashMap::new();
                $(
                    $(
                        insert_config_file_value(&mut out, $const_name, self.$field);
                    )+
                )+
                out
            }
        }
    };
}

config_schema! {
    group "Grok Responses" {
        /// Root URL, /v1 base URL, or full endpoint. Normalized to /v1 for Responses calls.
        GROK_API_URL {
            field: grok_api_url,
            type: String,
            toml: "grok_api_url",
            env: "GROK_SEARCH_URL",
            aliases: ["GROK_SEARCH_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "https://api.x.ai",
            sample: "\"https://api.x.ai\"",
        }
        /// Bearer token for the configured Grok/xAI-compatible Responses gateway.
        GROK_API_KEY {
            field: grok_api_key,
            type: String,
            toml: "grok_api_key",
            env: "GROK_SEARCH_API_KEY",
            aliases: ["GROK_SEARCH_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"xai-...\"",
        }
        /// Authentication mode. Use api_key for static keys or oauth for the local OAuth token file.
        GROK_AUTH_MODE {
            field: grok_auth_mode,
            type: String,
            toml: "grok_auth_mode",
            env: "GROK_SEARCH_AUTH_MODE",
            aliases: ["GROK_SEARCH_AUTH_MODE"],
            kind: String,
            redaction: None,
            default: "api_key",
            sample: "\"api_key\"",
        }
        /// Optional OAuth token file override.
        GROK_AUTH_FILE {
            field: grok_auth_file,
            type: String,
            toml: "grok_auth_file",
            env: "GROK_SEARCH_AUTH_FILE",
            aliases: ["GROK_SEARCH_AUTH_FILE"],
            kind: Path,
            redaction: Path,
            default: "default auth.json next to config.toml",
            sample: "\"C:\\\\Users\\\\you\\\\.config\\\\grok-search-rs\\\\auth.json\"",
        }
        /// Model sent in the Responses payload.
        GROK_MODEL {
            field: grok_model,
            type: String,
            toml: "grok_model",
            env: "GROK_SEARCH_MODEL",
            aliases: ["GROK_SEARCH_MODEL"],
            kind: String,
            redaction: None,
            default: "grok-4-1-fast-reasoning",
            sample: "\"grok-4-1-fast-reasoning\"",
        }
        /// Enables the upstream Responses web_search tool.
        WEB_SEARCH_ENABLED {
            field: web_search_enabled,
            type: bool,
            toml: "web_search_enabled",
            env: "GROK_SEARCH_WEB_SEARCH",
            aliases: ["GROK_SEARCH_WEB_SEARCH"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Enables the upstream Responses x_search tool when the gateway supports it.
        X_SEARCH_ENABLED {
            field: x_search_enabled,
            type: bool,
            toml: "x_search_enabled",
            env: "GROK_SEARCH_X_SEARCH",
            aliases: ["GROK_SEARCH_X_SEARCH"],
            kind: Bool,
            redaction: None,
            default: "false",
            sample: "false",
        }
    }

    group "Source providers" {
        /// Tavily API base URL.
        TAVILY_API_URL {
            field: tavily_api_url,
            type: String,
            toml: "tavily_api_url",
            env: "TAVILY_API_URL",
            aliases: ["TAVILY_API_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "https://api.tavily.com",
            sample: "\"https://api.tavily.com\"",
        }
        /// Tavily key for enrichment, fallback, fetch, and map.
        TAVILY_API_KEY {
            field: tavily_api_key,
            type: String,
            toml: "tavily_api_key",
            env: "TAVILY_API_KEY",
            aliases: ["TAVILY_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"tvly-...\"",
        }
        /// Optional Tavily enable override.
        TAVILY_ENABLED {
            field: tavily_enabled,
            type: bool,
            toml: "tavily_enabled",
            env: "TAVILY_ENABLED",
            aliases: ["TAVILY_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Firecrawl API base URL. Normalized to /v1.
        FIRECRAWL_API_URL {
            field: firecrawl_api_url,
            type: String,
            toml: "firecrawl_api_url",
            env: "FIRECRAWL_API_URL",
            aliases: ["FIRECRAWL_API_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "https://api.firecrawl.dev",
            sample: "\"https://api.firecrawl.dev\"",
        }
        /// Firecrawl key for fetch fallback and supplemental fallback sources.
        FIRECRAWL_API_KEY {
            field: firecrawl_api_key,
            type: String,
            toml: "firecrawl_api_key",
            env: "FIRECRAWL_API_KEY",
            aliases: ["FIRECRAWL_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"fc-...\"",
        }
        /// Optional Firecrawl enable override.
        FIRECRAWL_ENABLED {
            field: firecrawl_enabled,
            type: bool,
            toml: "firecrawl_enabled",
            env: "FIRECRAWL_ENABLED",
            aliases: ["FIRECRAWL_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Adds Tavily enrichment sources after a verifiable Grok result.
        DEFAULT_EXTRA_SOURCES {
            field: default_extra_sources,
            type: usize,
            toml: "default_extra_sources",
            env: "GROK_SEARCH_EXTRA_SOURCES",
            aliases: ["GROK_SEARCH_EXTRA_SOURCES"],
            kind: Usize,
            redaction: None,
            default: "3",
            sample: "3",
        }
        /// Number of fallback sources to cache when Grok is unverifiable.
        FALLBACK_SOURCES {
            field: fallback_sources,
            type: usize,
            toml: "fallback_sources",
            env: "GROK_SEARCH_FALLBACK_SOURCES",
            aliases: ["GROK_SEARCH_FALLBACK_SOURCES"],
            kind: Usize,
            redaction: None,
            default: "5",
            sample: "5",
        }
    }

    group "Runtime limits" {
        /// Default character cap on web_fetch content. Unset means no truncation.
        FETCH_MAX_CHARS {
            field: fetch_max_chars,
            type: usize,
            toml: "fetch_max_chars",
            env: "GROK_SEARCH_FETCH_MAX_CHARS",
            aliases: ["GROK_SEARCH_FETCH_MAX_CHARS"],
            kind: Usize,
            redaction: None,
            default: "unset",
            sample: "200000",
        }
        /// Maximum cached web_search sessions for get_sources.
        CACHE_SIZE {
            field: cache_size,
            type: usize,
            toml: "cache_size",
            env: "GROK_SEARCH_CACHE_SIZE",
            aliases: ["GROK_SEARCH_CACHE_SIZE"],
            kind: Usize,
            redaction: None,
            default: "256",
            sample: "256",
        }
        /// HTTP timeout in seconds for upstream requests.
        TIMEOUT_SECONDS {
            field: timeout_seconds,
            type: u64,
            toml: "timeout_seconds",
            env: "GROK_SEARCH_TIMEOUT_SECONDS",
            aliases: ["GROK_SEARCH_TIMEOUT_SECONDS"],
            kind: DurationSeconds,
            redaction: None,
            default: "60",
            sample: "60",
        }
        /// Proxy mode: auto, off, or an explicit proxy URL.
        PROXY {
            field: proxy,
            type: String,
            toml: "proxy",
            env: "GROK_SEARCH_PROXY",
            aliases: ["GROK_SEARCH_PROXY"],
            kind: String,
            redaction: ProxyUrl,
            default: "auto",
            sample: "\"auto\"",
        }
        /// Global upstream response body byte cap before parsing or trimming.
        MAX_RESPONSE_BYTES {
            field: max_response_bytes,
            type: usize,
            toml: "max_response_bytes",
            env: "GROK_SEARCH_MAX_RESPONSE_BYTES",
            aliases: ["GROK_SEARCH_MAX_RESPONSE_BYTES"],
            kind: Usize,
            redaction: None,
            default: "10485760",
            sample: "10485760",
        }
        /// Optional JSONL debug log path.
        DEBUG_LOG_PATH {
            field: debug_log_path,
            type: String,
            toml: "debug_log_path",
            env: "GROK_SEARCH_DEBUG_LOG_PATH",
            aliases: ["GROK_SEARCH_DEBUG_LOG_PATH"],
            kind: Path,
            redaction: Path,
            default: "unset",
            sample: "\"logs/grok-search-rs-debug.jsonl\"",
        }
    }

    group "OpenAI-compatible transport" {
        /// OpenAI-compatible chat-completions gateway base URL.
        OPENAI_COMPATIBLE_API_URL {
            field: openai_compatible_api_url,
            type: String,
            toml: "openai_compatible_api_url",
            env: "OPENAI_COMPATIBLE_API_URL",
            aliases: ["OPENAI_COMPATIBLE_API_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "unset",
            sample: "\"https://your-gateway/v1\"",
        }
        /// OpenAI-compatible gateway bearer token.
        OPENAI_COMPATIBLE_API_KEY {
            field: openai_compatible_api_key,
            type: String,
            toml: "openai_compatible_api_key",
            env: "OPENAI_COMPATIBLE_API_KEY",
            aliases: ["OPENAI_COMPATIBLE_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"sk-...\"",
        }
        /// OpenAI-compatible model name.
        OPENAI_COMPATIBLE_MODEL {
            field: openai_compatible_model,
            type: String,
            toml: "openai_compatible_model",
            env: "OPENAI_COMPATIBLE_MODEL",
            aliases: ["OPENAI_COMPATIBLE_MODEL"],
            kind: String,
            redaction: None,
            default: "falls back to grok_model",
            sample: "\"grok-4.3-fast\"",
        }
    }

    group "LLM providers" {
        /// Default LLM provider used by experimental PDF progressive reading.
        LLM_PROVIDER {
            field: llm_provider,
            type: String,
            toml: "llm_provider",
            env: "GROK_SEARCH_LLM_PROVIDER",
            aliases: ["GROK_SEARCH_LLM_PROVIDER"],
            kind: String,
            redaction: None,
            default: "minimax",
            sample: "\"minimax\"",
        }
        /// LLM API key for experimental PDF progressive reading.
        LLM_API_KEY {
            field: llm_api_key,
            type: String,
            toml: "llm_api_key",
            env: "GROK_SEARCH_LLM_API_KEY",
            aliases: ["GROK_SEARCH_LLM_API_KEY", "ANTHROPIC_API_KEY", "MINIMAX_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"sk-...\"",
        }
        /// Anthropic-compatible base URL for experimental PDF progressive reading.
        LLM_BASE_URL {
            field: llm_base_url,
            type: String,
            toml: "llm_base_url",
            env: "GROK_SEARCH_LLM_BASE_URL",
            aliases: ["GROK_SEARCH_LLM_BASE_URL", "ANTHROPIC_BASE_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "https://api.minimaxi.com/anthropic",
            sample: "\"https://api.minimaxi.com/anthropic\"",
        }
        /// Default LLM model for experimental PDF progressive reading.
        LLM_MODEL {
            field: llm_model,
            type: String,
            toml: "llm_model",
            env: "GROK_SEARCH_LLM_MODEL",
            aliases: ["GROK_SEARCH_LLM_MODEL", "ANTHROPIC_MODEL"],
            kind: String,
            redaction: None,
            default: "MiniMax-M3",
            sample: "\"MiniMax-M3\"",
        }
        /// Authentication scheme for Anthropic-compatible LLM calls.
        LLM_AUTH_SCHEME {
            field: llm_auth_scheme,
            type: String,
            toml: "llm_auth_scheme",
            env: "GROK_SEARCH_LLM_AUTH_SCHEME",
            aliases: ["GROK_SEARCH_LLM_AUTH_SCHEME"],
            kind: String,
            redaction: None,
            default: "bearer",
            sample: "\"bearer\"",
        }
    }

    group "Source extraction" {
        /// GitHub token for issue, PR, and repo metadata fetches.
        GITHUB_TOKEN {
            field: github_token,
            type: String,
            toml: "github_token",
            env: "GITHUB_TOKEN",
            aliases: ["GITHUB_TOKEN"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"ghp_...\"",
        }
        /// StackExchange answers rendered before folding.
        SOURCE_MAX_ANSWERS {
            field: source_max_answers,
            type: usize,
            toml: "source_max_answers",
            env: "GROK_SEARCH_SOURCE_MAX_ANSWERS",
            aliases: ["GROK_SEARCH_SOURCE_MAX_ANSWERS"],
            kind: Usize,
            redaction: None,
            default: "5",
            sample: "5",
        }
        /// GitHub and StackExchange comments rendered before folding.
        SOURCE_MAX_COMMENTS {
            field: source_max_comments,
            type: usize,
            toml: "source_max_comments",
            env: "GROK_SEARCH_SOURCE_MAX_COMMENTS",
            aliases: ["GROK_SEARCH_SOURCE_MAX_COMMENTS"],
            kind: Usize,
            redaction: None,
            default: "30",
            sample: "30",
        }
        /// Parallel source enrichments when web_search includes content.
        ENRICH_CONCURRENCY {
            field: enrich_concurrency,
            type: usize,
            toml: "enrich_concurrency",
            env: "GROK_SEARCH_ENRICH_CONCURRENCY",
            aliases: ["GROK_SEARCH_ENRICH_CONCURRENCY"],
            kind: Usize,
            redaction: None,
            default: "3",
            sample: "3",
        }
        /// Character cap per enriched source body.
        ENRICH_MAX_CHARS {
            field: enrich_max_chars,
            type: usize,
            toml: "enrich_max_chars",
            env: "GROK_SEARCH_ENRICH_MAX_CHARS",
            aliases: ["GROK_SEARCH_ENRICH_MAX_CHARS"],
            kind: Usize,
            redaction: None,
            default: "15000",
            sample: "15000",
        }
        /// Maximum sources carrying inline content per web_search response.
        MAX_INLINE_SOURCES {
            field: max_inline_sources,
            type: usize,
            toml: "max_inline_sources",
            env: "GROK_SEARCH_MAX_INLINE_SOURCES",
            aliases: ["GROK_SEARCH_MAX_INLINE_SOURCES"],
            kind: Usize,
            redaction: None,
            default: "5",
            sample: "5",
        }
        /// Whole-response character budget for web_search.
        RESPONSE_MAX_CHARS {
            field: response_max_chars,
            type: usize,
            toml: "response_max_chars",
            env: "GROK_SEARCH_RESPONSE_MAX_CHARS",
            aliases: ["GROK_SEARCH_RESPONSE_MAX_CHARS"],
            kind: Usize,
            redaction: None,
            default: "60000",
            sample: "60000",
        }
    }

    group "Academic and social search" {
        /// Enables the academic_* MCP tools.
        ACADEMIC_ENABLED {
            field: academic_enabled,
            type: bool,
            toml: "academic_enabled",
            env: "GROK_SEARCH_ACADEMIC_ENABLED",
            aliases: ["GROK_SEARCH_ACADEMIC_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Contact email for Unpaywall and polite academic API usage.
        ACADEMIC_EMAIL {
            field: academic_email,
            type: String,
            toml: "academic_email",
            env: "GROK_SEARCH_ACADEMIC_EMAIL",
            aliases: ["GROK_SEARCH_ACADEMIC_EMAIL", "UNPAYWALL_EMAIL"],
            kind: String,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"you@example.com\"",
        }
        /// Optional Semantic Scholar API key.
        SEMANTIC_SCHOLAR_API_KEY {
            field: semantic_scholar_api_key,
            type: String,
            toml: "semantic_scholar_api_key",
            env: "SEMANTIC_SCHOLAR_API_KEY",
            aliases: ["SEMANTIC_SCHOLAR_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"...\"",
        }
        /// Optional OpenAlex key. Comma-separated lists rotate keys round-robin.
        OPENALEX_API_KEY {
            field: openalex_api_key,
            type: String,
            toml: "openalex_api_key",
            env: "OPENALEX_API_KEY",
            aliases: ["OPENALEX_API_KEY", "GROK_SEARCH_OPENALEX_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"...\"",
        }
        /// Zhihu OpenAPI Access Secret for zhihu_search.
        ZHIHU_API_KEY {
            field: zhihu_api_key,
            type: String,
            toml: "zhihu_api_key",
            env: "ZHIHU_API_KEY",
            aliases: ["ZHIHU_ACCESS_SECRET", "ZHIHU_API_KEY"],
            kind: Secret,
            redaction: SecretStatus,
            default: "unset",
            sample: "\"...\"",
        }
        /// Zhihu OpenAPI base URL.
        ZHIHU_OPENAPI_BASE_URL {
            field: zhihu_openapi_base_url,
            type: String,
            toml: "zhihu_openapi_base_url",
            env: "ZHIHU_OPENAPI_BASE_URL",
            aliases: ["ZHIHU_OPENAPI_BASE_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "https://developer.zhihu.com",
            sample: "\"https://developer.zhihu.com\"",
        }
        /// Full Zhihu search endpoint override.
        ZHIHU_SEARCH_URL {
            field: zhihu_search_url,
            type: String,
            toml: "zhihu_search_url",
            env: "ZHIHU_ZHIHU_SEARCH_URL",
            aliases: ["ZHIHU_ZHIHU_SEARCH_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "unset",
            sample: "\"https://developer.zhihu.com/api/v1/content/zhihu_search\"",
        }
        /// Explicit opt-in for Sci-Hub fallback. Legal risk varies by jurisdiction.
        ACADEMIC_SCIHUB_ENABLED {
            field: academic_scihub_enabled,
            type: bool,
            toml: "academic_scihub_enabled",
            env: "GROK_SEARCH_ACADEMIC_SCIHUB_ENABLED",
            aliases: ["GROK_SEARCH_ACADEMIC_SCIHUB_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "false",
            sample: "false",
        }
        /// Sci-Hub base URL, used only when academic_scihub_enabled is true.
        ACADEMIC_SCIHUB_BASE_URL {
            field: academic_scihub_base_url,
            type: String,
            toml: "academic_scihub_base_url",
            env: "GROK_SEARCH_ACADEMIC_SCIHUB_BASE_URL",
            aliases: ["GROK_SEARCH_ACADEMIC_SCIHUB_BASE_URL"],
            kind: UrlBase,
            redaction: Url,
            default: "unset",
            sample: "\"https://...\"",
        }
        /// Enables IEEE/ACM institutional PDF fallback.
        ACADEMIC_INSTITUTIONAL_ENABLED {
            field: academic_institutional_enabled,
            type: bool,
            toml: "academic_institutional_enabled",
            env: "GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ENABLED",
            aliases: ["GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Allows invalid TLS certificates only for private institutional fallback routes.
        ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS {
            field: academic_institutional_accept_invalid_certs,
            type: bool,
            toml: "academic_institutional_accept_invalid_certs",
            env: "GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS",
            aliases: ["GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS"],
            kind: Bool,
            redaction: None,
            default: "false",
            sample: "false",
        }
        /// Probes direct and discovered proxy routes for IEEE/ACM access.
        ACADEMIC_INSTITUTIONAL_PROBE {
            field: academic_institutional_probe,
            type: bool,
            toml: "academic_institutional_probe",
            env: "GROK_SEARCH_ACADEMIC_INSTITUTIONAL_PROBE",
            aliases: ["GROK_SEARCH_ACADEMIC_INSTITUTIONAL_PROBE"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Maximum PDF bytes downloaded for academic PDF read, parse, and download flows.
        ACADEMIC_MAX_PDF_BYTES {
            field: academic_max_pdf_bytes,
            type: usize,
            toml: "academic_max_pdf_bytes",
            env: "GROK_SEARCH_ACADEMIC_MAX_PDF_BYTES",
            aliases: ["GROK_SEARCH_ACADEMIC_MAX_PDF_BYTES"],
            kind: Usize,
            redaction: None,
            default: "52428800",
            sample: "52428800",
        }
        /// Character cap for parsed PDF output.
        ACADEMIC_PDF_MAX_CHARS {
            field: academic_pdf_max_chars,
            type: usize,
            toml: "academic_pdf_max_chars",
            env: "GROK_SEARCH_ACADEMIC_PDF_MAX_CHARS",
            aliases: ["GROK_SEARCH_ACADEMIC_PDF_MAX_CHARS"],
            kind: Usize,
            redaction: None,
            default: "unset",
            sample: "200000",
        }
        /// Enables persistent KV cache for LLM progressive PDF reading structures.
        PROGRESSIVE_CACHE_ENABLED {
            field: progressive_cache_enabled,
            type: bool,
            toml: "progressive_cache_enabled",
            env: "GROK_SEARCH_PROGRESSIVE_CACHE_ENABLED",
            aliases: ["GROK_SEARCH_PROGRESSIVE_CACHE_ENABLED"],
            kind: Bool,
            redaction: None,
            default: "true",
            sample: "true",
        }
        /// Persistent KV cache path for LLM progressive PDF reading structures.
        PROGRESSIVE_CACHE_PATH {
            field: progressive_cache_path,
            type: String,
            toml: "progressive_cache_path",
            env: "GROK_SEARCH_PROGRESSIVE_CACHE_PATH",
            aliases: ["GROK_SEARCH_PROGRESSIVE_CACHE_PATH"],
            kind: Path,
            redaction: Path,
            default: "default progressive-cache.redb next to config.toml",
            sample: "\"/path/to/progressive-cache.redb\"",
        }
        /// Seconds before progressive reading cache entries expire.
        PROGRESSIVE_CACHE_TTL_SECONDS {
            field: progressive_cache_ttl_seconds,
            type: u64,
            toml: "progressive_cache_ttl_seconds",
            env: "GROK_SEARCH_PROGRESSIVE_CACHE_TTL_SECONDS",
            aliases: ["GROK_SEARCH_PROGRESSIVE_CACHE_TTL_SECONDS"],
            kind: DurationSeconds,
            redaction: None,
            default: "2592000",
            sample: "2592000",
        }
        /// Maximum progressive reading cache entries retained after writes.
        PROGRESSIVE_CACHE_MAX_ENTRIES {
            field: progressive_cache_max_entries,
            type: usize,
            toml: "progressive_cache_max_entries",
            env: "GROK_SEARCH_PROGRESSIVE_CACHE_MAX_ENTRIES",
            aliases: ["GROK_SEARCH_PROGRESSIVE_CACHE_MAX_ENTRIES"],
            kind: Usize,
            redaction: None,
            default: "512",
            sample: "512",
        }
        /// Default model for LLM progressive PDF reading when the tool does not pass one.
        PROGRESSIVE_DEFAULT_MODEL {
            field: progressive_default_model,
            type: String,
            toml: "progressive_default_model",
            env: "GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL",
            aliases: ["GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL"],
            kind: String,
            redaction: None,
            default: "MiniMax-M3",
            sample: "\"MiniMax-M3\"",
        }
        /// Default profile for LLM progressive PDF reading.
        PROGRESSIVE_DEFAULT_PROFILE {
            field: progressive_default_profile,
            type: String,
            toml: "progressive_default_profile",
            env: "GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE",
            aliases: ["GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE"],
            kind: String,
            redaction: None,
            default: "balanced",
            sample: "\"balanced\"",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn config_schema_keys_are_unique_and_aliases_include_canonical_env() {
        let mut toml_keys = HashSet::new();
        let mut canonical_envs = HashSet::new();
        for key in CONFIG_KEYS {
            assert!(
                toml_keys.insert(key.toml_key),
                "duplicate TOML key {}",
                key.toml_key
            );
            assert!(
                canonical_envs.insert(key.canonical_env),
                "duplicate canonical env {}",
                key.canonical_env
            );
            assert!(
                key.env_aliases.contains(&key.canonical_env),
                "{} aliases must include canonical env {}",
                key.toml_key,
                key.canonical_env
            );
        }
    }

    #[test]
    fn config_schema_records_key_aliases_and_redaction() {
        assert_eq!(
            ZHIHU_API_KEY.env_aliases,
            &["ZHIHU_ACCESS_SECRET", "ZHIHU_API_KEY"]
        );
        assert_eq!(
            OPENALEX_API_KEY.env_aliases,
            &["OPENALEX_API_KEY", "GROK_SEARCH_OPENALEX_API_KEY"]
        );
        assert_eq!(ZHIHU_API_KEY.redaction, RedactionKind::SecretStatus);
        assert_eq!(ZHIHU_SEARCH_URL.redaction, RedactionKind::Url);
        assert_eq!(PROXY.redaction, RedactionKind::ProxyUrl);
        assert_eq!(TIMEOUT_SECONDS.kind, ConfigValueKind::DurationSeconds);
    }
}
