use std::fs;

use grok_search_config as config;
use grok_search_config::{AuthMode, Config, InitOutcome, Transport};
use tempfile::tempdir;

#[test]
fn config_reads_grok_search_responses_defaults() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("TAVILY_API_KEY", "tvly-test-key"),
    ]);

    assert_eq!(cfg.grok_api_url, "https://api.x.ai/v1");
    assert_eq!(cfg.grok_model, "grok-4-1-fast-reasoning");
    assert!(cfg.web_search_enabled);
    assert!(!cfg.x_search_enabled);
    assert_eq!(cfg.tavily_api_url, "https://api.tavily.com");
    assert!(cfg.tavily_enabled);
    assert_eq!(cfg.default_extra_sources, 3);
    assert_eq!(cfg.fallback_sources, 5);
    assert_eq!(cfg.timeout.as_secs(), 60);
    assert_eq!(cfg.proxy, "auto");
    assert_eq!(cfg.grok_auth_mode, AuthMode::ApiKey);
    assert_eq!(cfg.max_response_bytes, 10 * 1024 * 1024);
}

#[test]
fn config_reads_oauth_auth_mode_from_env() {
    let cfg = Config::from_env_map([("GROK_SEARCH_AUTH_MODE", "oauth")]);

    assert_eq!(cfg.grok_auth_mode, AuthMode::OAuth);
    assert_eq!(cfg.transport, Transport::Responses);
}

#[test]
fn unknown_auth_mode_falls_back_to_api_key() {
    let cfg = Config::from_env_map([("GROK_SEARCH_AUTH_MODE", "something_else")]);

    assert_eq!(cfg.grok_auth_mode, AuthMode::ApiKey);
}

#[test]
fn oauth_transport_wins_over_openai_compatible_config() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_AUTH_MODE", "oauth"),
        ("OPENAI_COMPATIBLE_API_URL", "https://example.com/v1"),
        ("OPENAI_COMPATIBLE_API_KEY", "sk-fake"),
    ]);

    assert_eq!(cfg.grok_auth_mode, AuthMode::OAuth);
    assert_eq!(cfg.transport, Transport::Responses);
}

#[test]
fn config_reads_auth_file_override() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_AUTH_MODE", "oauth"),
        (
            "GROK_SEARCH_AUTH_FILE",
            "C:\\Users\\chen\\.config\\grok-search-rs\\auth.json",
        ),
    ]);

    assert_eq!(cfg.grok_auth_mode, AuthMode::OAuth);
    assert_eq!(
        cfg.grok_auth_file,
        Some(std::path::PathBuf::from(
            "C:\\Users\\chen\\.config\\grok-search-rs\\auth.json"
        ))
    );
}

#[test]
fn config_normalizes_grok_search_url_to_v1_base() {
    let cases = [
        ("https://api.modelverse.cn", "https://api.modelverse.cn/v1"),
        ("https://api.modelverse.cn/", "https://api.modelverse.cn/v1"),
        (
            "https://api.modelverse.cn/v1",
            "https://api.modelverse.cn/v1",
        ),
        (
            "https://api.modelverse.cn/v1/responses",
            "https://api.modelverse.cn/v1",
        ),
    ];

    for (input, expected) in cases {
        let cfg = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "grok-test-key"),
            ("GROK_SEARCH_URL", input),
        ]);
        assert_eq!(cfg.grok_api_url, expected);
    }
}

#[test]
fn config_enables_x_search_only_when_configured() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("GROK_SEARCH_X_SEARCH", "true"),
    ]);

    assert!(cfg.x_search_enabled);
}

#[test]
fn config_reads_firecrawl_settings() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("FIRECRAWL_API_KEY", "fc-test-key"),
        ("FIRECRAWL_API_URL", "https://firecrawl.example/v1"),
        ("FIRECRAWL_ENABLED", "true"),
    ]);

    assert_eq!(cfg.firecrawl_api_url, "https://firecrawl.example/v1");
    assert_eq!(cfg.firecrawl_api_key.as_deref(), Some("fc-test-key"));
    assert!(cfg.firecrawl_enabled);
}

#[test]
fn config_redacts_grok_tavily_and_firecrawl_keys() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-1234567890"),
        ("TAVILY_API_KEY", "tvly-abcdefghi"),
        ("FIRECRAWL_API_KEY", "fc-abcdefghi"),
        ("GROK_SEARCH_LLM_API_KEY", "llm-secret-token"),
        ("GROK_SEARCH_MCP_HTTP_AUTH_TOKEN", "mcp-secret-token"),
        (
            "GROK_SEARCH_LLM_BASE_URL",
            "https://user:pass@example.com/anthropic?token=secret",
        ),
    ]);

    let info = cfg.redacted_diagnostics();
    assert!(info.contains("grok_api_key=set"));
    assert!(info.contains("tavily_api_key=set"));
    assert!(info.contains("firecrawl_api_key=set"));
    assert!(info.contains("llm_api_key=set"));
    assert!(info.contains("mcp_http_auth_token=set"));
    assert!(!info.contains("tvly"));
    assert!(!info.contains("fc-"));
    assert!(!info.contains("grok-1234567890"));
    assert!(!info.contains("llm-secret-token"));
    assert!(!info.contains("mcp-secret-token"));
    assert!(!info.contains("user:pass"));
    assert!(!info.contains("token=secret"));
    assert!(!info.contains("1234567890"));
    assert!(!info.contains("abcdefghi"));
}

#[test]
fn config_reads_mcp_http_defaults_and_env_overrides() {
    let defaults = Config::from_env_map([] as [(&str, &str); 0]);
    assert_eq!(defaults.mcp_http_bind, "127.0.0.1:8787");
    assert_eq!(defaults.mcp_http_path, "/mcp");
    assert_eq!(defaults.mcp_http_auth_token, None);
    assert_eq!(defaults.mcp_http_allow_origin, None);

    let cfg = Config::from_env_map([
        ("GROK_SEARCH_MCP_HTTP_BIND", "127.0.0.1:9999"),
        ("GROK_SEARCH_MCP_HTTP_PATH", "/custom-mcp"),
        ("GROK_SEARCH_MCP_HTTP_AUTH_TOKEN", "secret"),
        ("GROK_SEARCH_MCP_HTTP_ALLOW_ORIGIN", "http://127.0.0.1:3000"),
    ]);
    assert_eq!(cfg.mcp_http_bind, "127.0.0.1:9999");
    assert_eq!(cfg.mcp_http_path, "/custom-mcp");
    assert_eq!(cfg.mcp_http_auth_token.as_deref(), Some("secret"));
    assert_eq!(
        cfg.mcp_http_allow_origin.as_deref(),
        Some("http://127.0.0.1:3000")
    );
}

#[test]
fn config_reads_extra_sources_and_fallback_sources_from_env() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("GROK_SEARCH_EXTRA_SOURCES", "3"),
        ("GROK_SEARCH_FALLBACK_SOURCES", "7"),
    ]);

    assert_eq!(cfg.default_extra_sources, 3);
    assert_eq!(cfg.fallback_sources, 7);
}

#[test]
fn config_reads_timeout_seconds() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("GROK_SEARCH_TIMEOUT_SECONDS", "90"),
    ]);

    assert_eq!(cfg.timeout.as_secs(), 90);
}

#[test]
fn config_reads_proxy_mode() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("GROK_SEARCH_PROXY", "off"),
    ]);

    assert_eq!(cfg.proxy, "off");
}

#[test]
fn invalid_source_counts_fall_back_to_safe_defaults() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_API_KEY", "grok-test-key"),
        ("GROK_SEARCH_EXTRA_SOURCES", "not-a-number"),
        ("GROK_SEARCH_FALLBACK_SOURCES", "not-a-number"),
    ]);

    assert_eq!(cfg.default_extra_sources, 3);
    assert_eq!(cfg.fallback_sources, 5);
    assert_eq!(cfg.timeout.as_secs(), 60);
}

#[test]
fn config_file_supplies_values_when_env_absent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        r#"
grok_api_key = "xai-from-file"
grok_model   = "grok-5-test"
tavily_api_key = "tvly-from-file"
openalex_api_key = "oa-file-a,oa-file-b"
zhihu_api_key = "zhihu-from-file"
default_extra_sources = 7
timeout_seconds = 42
proxy = "http://file-user:file-pass@127.0.0.1:7890"
"#,
    )
    .unwrap();

    let cfg = Config::load_from([("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string())]);

    assert_eq!(cfg.grok_api_key.as_deref(), Some("xai-from-file"));
    assert_eq!(cfg.grok_model, "grok-5-test");
    assert_eq!(cfg.tavily_api_key.as_deref(), Some("tvly-from-file"));
    assert_eq!(cfg.openalex_api_key.as_deref(), Some("oa-file-a,oa-file-b"));
    assert_eq!(cfg.zhihu_api_key.as_deref(), Some("zhihu-from-file"));
    assert_eq!(cfg.default_extra_sources, 7);
    assert_eq!(cfg.timeout.as_secs(), 42);
    assert_eq!(cfg.proxy, "http://file-user:file-pass@127.0.0.1:7890");
}

#[test]
fn env_overrides_config_file_values() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        r#"
grok_model = "model-from-file"
default_extra_sources = 7
"#,
    )
    .unwrap();

    let cfg = Config::load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("GROK_SEARCH_API_KEY", "grok-env-key".into()),
        ("GROK_SEARCH_MODEL", "model-from-env".into()),
        ("GROK_SEARCH_EXTRA_SOURCES", "2".into()),
        ("GROK_SEARCH_PROXY", "off".into()),
    ]);

    assert_eq!(cfg.grok_model, "model-from-env");
    assert_eq!(cfg.default_extra_sources, 2);
    assert_eq!(cfg.grok_api_key.as_deref(), Some("grok-env-key"));
    assert_eq!(cfg.proxy, "off");
}

#[test]
fn zhihu_access_secret_and_api_key_env_override_config_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, r#"zhihu_api_key = "zhihu-from-file""#).unwrap();

    let cfg = Config::load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("ZHIHU_API_KEY", "zhihu-from-env".into()),
    ]);
    assert_eq!(cfg.zhihu_api_key.as_deref(), Some("zhihu-from-env"));

    let cfg = Config::load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("ZHIHU_API_KEY", "zhihu-from-env".into()),
        ("ZHIHU_ACCESS_SECRET", "zhihu-access-secret".into()),
    ]);
    assert_eq!(cfg.zhihu_api_key.as_deref(), Some("zhihu-access-secret"));
}

#[test]
fn llm_config_reads_canonical_and_legacy_env_aliases() {
    let cfg = Config::from_env_map([
        ("GROK_SEARCH_LLM_API_KEY", "llm-key"),
        ("GROK_SEARCH_LLM_BASE_URL", "https://llm.example/anthropic"),
        ("GROK_SEARCH_LLM_MODEL", "MiniMax-M3"),
        ("GROK_SEARCH_LLM_AUTH_SCHEME", "both"),
    ]);

    assert_eq!(cfg.llm_api_key.as_deref(), Some("llm-key"));
    assert_eq!(cfg.llm_base_url, "https://llm.example/anthropic");
    assert_eq!(cfg.llm_model, "MiniMax-M3");
    assert_eq!(cfg.llm_auth_scheme, "both");
    assert_eq!(cfg.progressive_default_model, "MiniMax-M3");

    let cfg = Config::from_env_map([
        ("ANTHROPIC_API_KEY", "anthropic-key"),
        ("MINIMAX_API_KEY", "minimax-key"),
        ("ANTHROPIC_BASE_URL", "https://legacy.example/anthropic"),
        ("ANTHROPIC_MODEL", "legacy-model"),
    ]);

    assert_eq!(cfg.llm_api_key.as_deref(), Some("anthropic-key"));
    assert_eq!(cfg.llm_base_url, "https://legacy.example/anthropic");
    assert_eq!(cfg.llm_model, "legacy-model");
}

#[test]
fn llm_env_overrides_config_file_even_for_legacy_alias() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        r#"
llm_api_key = "file-key"
llm_model = "file-model"
"#,
    )
    .unwrap();

    let cfg = Config::load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("ANTHROPIC_API_KEY", "env-key".into()),
        ("ANTHROPIC_MODEL", "env-model".into()),
    ]);

    assert_eq!(cfg.llm_api_key.as_deref(), Some("env-key"));
    assert_eq!(cfg.llm_model, "env-model");
}

#[test]
fn progressive_cache_path_defaults_next_to_config() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();

    let cfg = Config::try_load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("GROK_SEARCH_API_KEY", "fake".into()),
    ])
    .unwrap();

    assert_eq!(
        cfg.progressive_cache_path,
        path.with_file_name("progressive-cache.redb")
    );
}

#[test]
fn academic_pdf_cache_path_defaults_next_to_config() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();

    let cfg = Config::try_load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("GROK_SEARCH_API_KEY", "fake".into()),
    ])
    .unwrap();

    assert_eq!(
        cfg.academic_pdf_cache_path,
        path.with_file_name("academic-pdf-cache.redb")
    );
}

#[test]
fn progressive_cache_path_env_overrides_default() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let cache = dir.path().join("custom.redb");
    fs::write(&path, "").unwrap();

    let cfg = Config::try_load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("GROK_SEARCH_API_KEY", "fake".into()),
        (
            "GROK_SEARCH_PROGRESSIVE_CACHE_PATH",
            cache.to_string_lossy().to_string(),
        ),
    ])
    .unwrap();

    assert_eq!(cfg.progressive_cache_path, cache);
}

#[test]
fn academic_pdf_cache_path_env_overrides_default() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let cache = dir.path().join("pdf-cache.redb");
    fs::write(&path, "").unwrap();

    let cfg = Config::try_load_from([
        ("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string()),
        ("GROK_SEARCH_API_KEY", "fake".into()),
        (
            "GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH",
            cache.to_string_lossy().to_string(),
        ),
    ])
    .unwrap();

    assert_eq!(cfg.academic_pdf_cache_path, cache);
}

#[test]
fn missing_config_file_falls_back_to_env_and_defaults() {
    let dir = tempdir().unwrap();
    let nonexistent = dir.path().join("nope.toml");

    let cfg = Config::load_from([
        (
            "GROK_SEARCH_CONFIG",
            nonexistent.to_string_lossy().to_string(),
        ),
        ("GROK_SEARCH_API_KEY", "grok-env-key".into()),
    ]);

    assert_eq!(cfg.grok_api_key.as_deref(), Some("grok-env-key"));
    assert_eq!(cfg.grok_model, "grok-4-1-fast-reasoning");
    assert_eq!(cfg.default_extra_sources, 3);
}

#[test]
fn config_file_supports_all_documented_keys() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(
        &path,
        r#"
grok_api_url          = "https://api.modelverse.cn"
grok_api_key          = "xai-full"
grok_auth_mode        = "oauth"
grok_auth_file        = 'C:\Users\chen\.config\grok-search-rs\auth.json'
grok_model            = "grok-9000"
web_search_enabled    = false
x_search_enabled      = true
tavily_api_url        = "https://tavily.example"
tavily_api_key        = "tvly-full"
tavily_enabled        = false
firecrawl_api_url     = "https://firecrawl.example"
firecrawl_api_key     = "fc-full"
firecrawl_enabled     = false
academic_email        = "person@example.com"
semantic_scholar_api_key = "s2-full"
openalex_api_key      = "oa-full-a,oa-full-b"
zhihu_api_key         = "zhihu-full"
zhihu_openapi_base_url = "https://developer.zhihu.example"
zhihu_search_url      = "https://gateway.example/zhihu_search"
llm_provider          = "minimax"
llm_api_key           = "llm-full"
llm_base_url          = "https://llm.example/anthropic"
llm_model             = "MiniMax-M3"
llm_auth_scheme       = "bearer"
progressive_cache_enabled = false
progressive_cache_path = "custom-progressive.redb"
progressive_cache_ttl_seconds = 60
progressive_cache_max_entries = 7
academic_pdf_cache_enabled = false
academic_pdf_cache_path = "custom-pdf-cache.redb"
academic_pdf_cache_ttl_seconds = 120
academic_pdf_cache_max_entries = 11
academic_pdf_cache_max_bytes = 123456
mcp_http_bind = "127.0.0.1:9999"
mcp_http_path = "/custom-mcp"
mcp_http_auth_token = "mcp-secret"
mcp_http_allow_origin = "http://127.0.0.1:3000"
default_extra_sources = 4
fallback_sources      = 9
fetch_max_chars       = 12345
cache_size            = 128
timeout_seconds       = 30
max_response_bytes    = 2097152
"#,
    )
    .unwrap();

    let cfg = Config::load_from([("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string())]);

    assert_eq!(cfg.grok_api_url, "https://api.modelverse.cn/v1");
    assert_eq!(cfg.grok_api_key.as_deref(), Some("xai-full"));
    assert_eq!(cfg.grok_auth_mode, AuthMode::OAuth);
    assert_eq!(
        cfg.grok_auth_file,
        Some(std::path::PathBuf::from(
            "C:\\Users\\chen\\.config\\grok-search-rs\\auth.json"
        ))
    );
    assert_eq!(cfg.grok_model, "grok-9000");
    assert!(!cfg.web_search_enabled);
    assert!(cfg.x_search_enabled);
    assert_eq!(cfg.tavily_api_url, "https://tavily.example");
    assert_eq!(cfg.tavily_api_key.as_deref(), Some("tvly-full"));
    assert!(!cfg.tavily_enabled);
    assert_eq!(cfg.firecrawl_api_url, "https://firecrawl.example/v1");
    assert_eq!(cfg.firecrawl_api_key.as_deref(), Some("fc-full"));
    assert!(!cfg.firecrawl_enabled);
    assert_eq!(cfg.academic_email.as_deref(), Some("person@example.com"));
    assert_eq!(cfg.semantic_scholar_api_key.as_deref(), Some("s2-full"));
    assert_eq!(cfg.openalex_api_key.as_deref(), Some("oa-full-a,oa-full-b"));
    assert_eq!(cfg.zhihu_api_key.as_deref(), Some("zhihu-full"));
    assert_eq!(
        cfg.zhihu_openapi_base_url,
        "https://developer.zhihu.example"
    );
    assert_eq!(
        cfg.zhihu_search_url.as_deref(),
        Some("https://gateway.example/zhihu_search")
    );
    assert_eq!(cfg.llm_provider, "minimax");
    assert_eq!(cfg.llm_api_key.as_deref(), Some("llm-full"));
    assert_eq!(cfg.llm_base_url, "https://llm.example/anthropic");
    assert_eq!(cfg.llm_model, "MiniMax-M3");
    assert_eq!(cfg.llm_auth_scheme, "bearer");
    assert_eq!(cfg.default_extra_sources, 4);
    assert_eq!(cfg.fallback_sources, 9);
    assert_eq!(cfg.fetch_max_chars, Some(12345));
    assert_eq!(cfg.cache_size, 128);
    assert_eq!(cfg.timeout.as_secs(), 30);
    assert_eq!(cfg.max_response_bytes, 2 * 1024 * 1024);
    assert!(!cfg.progressive_cache_enabled);
    assert_eq!(
        cfg.progressive_cache_path,
        std::path::PathBuf::from("custom-progressive.redb")
    );
    assert_eq!(cfg.progressive_cache_ttl_seconds, 60);
    assert_eq!(cfg.progressive_cache_max_entries, 7);
    assert!(!cfg.academic_pdf_cache_enabled);
    assert_eq!(
        cfg.academic_pdf_cache_path,
        std::path::PathBuf::from("custom-pdf-cache.redb")
    );
    assert_eq!(cfg.academic_pdf_cache_ttl_seconds, 120);
    assert_eq!(cfg.academic_pdf_cache_max_entries, 11);
    assert_eq!(cfg.academic_pdf_cache_max_bytes, 123456);
    assert_eq!(cfg.mcp_http_bind, "127.0.0.1:9999");
    assert_eq!(cfg.mcp_http_path, "/custom-mcp");
    assert_eq!(cfg.mcp_http_auth_token.as_deref(), Some("mcp-secret"));
    assert_eq!(
        cfg.mcp_http_allow_origin.as_deref(),
        Some("http://127.0.0.1:3000")
    );
}

#[test]
fn write_template_creates_file_then_is_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("config.toml");

    let first = config::write_template(&path).unwrap();
    assert_eq!(first, InitOutcome::Created);
    assert!(path.exists(), "template file must exist after first call");

    let body = fs::read_to_string(&path).unwrap();
    assert!(
        body.contains("grok-search-rs global configuration"),
        "template header must be present"
    );

    let second = config::write_template(&path).unwrap();
    assert_eq!(
        second,
        InitOutcome::AlreadyExists,
        "second call must not overwrite"
    );
    // Body unchanged after second call.
    assert_eq!(fs::read_to_string(&path).unwrap(), body);
}

#[test]
fn fresh_template_does_not_override_defaults_or_supply_credentials() {
    // The whole point of commenting out every key in the template is so an
    // un-edited scaffold behaves identically to "no config file at all".
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    config::write_template(&path).unwrap();

    let cfg = Config::load_from([("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string())]);

    assert!(
        cfg.grok_api_key.is_none(),
        "empty template must NOT set grok_api_key (else onboarding guide gets bypassed)"
    );
    assert!(cfg.tavily_api_key.is_none());
    assert_eq!(cfg.grok_model, "grok-4-1-fast-reasoning");
    assert_eq!(cfg.grok_api_url, "https://api.x.ai/v1");
    assert_eq!(cfg.default_extra_sources, 3);
    assert_eq!(cfg.fallback_sources, 5);
    assert_eq!(cfg.cache_size, 256);
    assert_eq!(cfg.timeout.as_secs(), 60);
    assert!(cfg.web_search_enabled);
    assert!(!cfg.x_search_enabled);
    assert!(cfg.tavily_enabled);
    assert!(cfg.firecrawl_enabled);
}

#[test]
fn config_path_honors_explicit_env_override() {
    let path = config::config_path();
    // Test runs on macOS/Linux where HOME is set; just sanity-check the shape.
    if let Some(p) = path {
        assert!(p.ends_with("config.toml"));
    }
}

#[test]
fn config_path_explicit_override_wins_over_home() {
    let path = config::config_path_for([
        ("GROK_SEARCH_CONFIG", "/tmp/custom/grok.toml"),
        ("HOME", "/home/ignored"),
        ("USERPROFILE", "C:\\Users\\ignored"),
    ])
    .expect("explicit override must resolve");
    assert_eq!(path, std::path::PathBuf::from("/tmp/custom/grok.toml"));
}

#[test]
fn config_path_uses_home_on_unix_layout() {
    let path =
        config::config_path_for([("HOME", "/home/alice")]).expect("HOME must produce a path");
    let expected = std::path::PathBuf::from("/home/alice")
        .join(".config")
        .join("grok-search-rs")
        .join("config.toml");
    assert_eq!(path, expected);
}

#[test]
fn config_path_falls_back_to_userprofile_when_home_missing() {
    let path = config::config_path_for([("USERPROFILE", "C:\\Users\\chen")])
        .expect("USERPROFILE must produce a path on Windows-style env");
    let expected = std::path::PathBuf::from("C:\\Users\\chen")
        .join(".config")
        .join("grok-search-rs")
        .join("config.toml");
    assert_eq!(path, expected);
}

#[test]
fn config_path_prefers_home_over_userprofile_when_both_set() {
    let path =
        config::config_path_for([("HOME", "/home/alice"), ("USERPROFILE", "C:\\Users\\chen")])
            .expect("must resolve");
    assert!(
        path.starts_with("/home/alice"),
        "HOME should win, got {}",
        path.display()
    );
}

#[test]
fn auth_path_uses_config_sibling_by_default() {
    let path =
        config::auth_path_for([("HOME", "/home/alice")]).expect("HOME must produce auth path");
    let expected = std::path::PathBuf::from("/home/alice")
        .join(".config")
        .join("grok-search-rs")
        .join("auth.json");
    assert_eq!(path, expected);
}

#[test]
fn auth_path_falls_back_to_userprofile_when_home_missing() {
    let path = config::auth_path_for([("USERPROFILE", "C:\\Users\\chen")])
        .expect("USERPROFILE must produce auth path");
    let expected = std::path::PathBuf::from("C:\\Users\\chen")
        .join(".config")
        .join("grok-search-rs")
        .join("auth.json");
    assert_eq!(path, expected);
}

#[test]
fn auth_path_honors_explicit_override() {
    let path = config::auth_path_for([
        ("GROK_SEARCH_AUTH_FILE", "/tmp/auth.json"),
        ("HOME", "/home/ignored"),
    ])
    .expect("explicit override must resolve");
    assert_eq!(path, std::path::PathBuf::from("/tmp/auth.json"));
}

#[test]
fn config_path_none_when_no_env_set() {
    let env: [(&str, &str); 0] = [];
    assert!(config::config_path_for(env).is_none());
}

#[test]
fn response_budget_defaults_and_env_overrides() {
    let defaults = Config::from_env_map([] as [(&str, &str); 0]);
    assert_eq!(defaults.max_inline_sources, 5);
    assert_eq!(defaults.response_max_chars, 60_000);
    assert_eq!(defaults.max_response_bytes, 10 * 1024 * 1024);
    assert_eq!(defaults.debug_log_path, None);

    let overridden = Config::from_env_map([
        ("GROK_SEARCH_MAX_INLINE_SOURCES", "2"),
        ("GROK_SEARCH_RESPONSE_MAX_CHARS", "30000"),
        ("GROK_SEARCH_MAX_RESPONSE_BYTES", "123456"),
        ("GROK_SEARCH_DEBUG_LOG_PATH", "logs/debug.jsonl"),
    ]);
    assert_eq!(overridden.max_inline_sources, 2);
    assert_eq!(overridden.response_max_chars, 30_000);
    assert_eq!(overridden.max_response_bytes, 123456);
    assert_eq!(
        overridden.debug_log_path.as_deref(),
        Some(std::path::Path::new("logs/debug.jsonl"))
    );
}

#[test]
fn max_inline_sources_allows_zero_and_clamps_large_values() {
    let disabled = Config::from_env_map([("GROK_SEARCH_MAX_INLINE_SOURCES", "0")]);
    assert_eq!(disabled.max_inline_sources, 0);

    let large = Config::from_env_map([("GROK_SEARCH_MAX_INLINE_SOURCES", "999")]);
    assert_eq!(
        large.max_inline_sources,
        grok_search_config::MAX_INLINE_SOURCES_LIMIT
    );
}

#[test]
fn debug_log_path_loads_from_toml_and_is_created() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let log_path = dir.path().join("nested").join("debug.jsonl");
    fs::write(
        &config_path,
        format!("debug_log_path = {:?}\n", log_path.display().to_string()),
    )
    .unwrap();

    let cfg = Config::try_load_from([(
        "GROK_SEARCH_CONFIG",
        config_path.to_string_lossy().to_string(),
    )])
    .expect("debug log path should be valid");

    assert_eq!(cfg.debug_log_path.as_deref(), Some(log_path.as_path()));
    assert!(log_path.exists());
}

#[test]
fn malformed_explicit_config_file_returns_error_in_fallible_loader() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "not valid toml = [").unwrap();

    let err = Config::try_load_from([("GROK_SEARCH_CONFIG", path.to_string_lossy().to_string())])
        .expect_err("malformed explicit config must fail");
    assert!(err.to_string().contains("parse config"), "{err}");
}

#[test]
fn institutional_invalid_certs_default_false_and_explicit_true() {
    let defaults = Config::from_env_map([] as [(&str, &str); 0]);
    assert!(!defaults.academic_institutional_accept_invalid_certs);

    let enabled = Config::from_env_map([(
        "GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS",
        "true",
    )]);
    assert!(enabled.academic_institutional_accept_invalid_certs);
}
