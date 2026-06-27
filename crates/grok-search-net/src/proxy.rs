use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

use reqwest::Client;
use serde_json::json;
use url::Url;

use crate::http::{build_client_direct, build_client_with_proxy};
use grok_search_config::{AuthMode, Config, Transport};
use grok_search_types::Result as GrokResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyDiagnostics {
    pub mode: String,
    pub status: String,
    pub source: String,
    pub url_redacted: Option<String>,
    pub detail: String,
    pub checked_urls: Vec<String>,
}

impl ProxyDiagnostics {
    pub fn direct(
        mode: impl Into<String>,
        detail: impl Into<String>,
        checked_urls: Vec<String>,
    ) -> Self {
        Self {
            mode: mode.into(),
            status: "direct".to_string(),
            source: "none".to_string(),
            url_redacted: None,
            detail: detail.into(),
            checked_urls,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "mode": self.mode,
            "status": self.status,
            "source": self.source,
            "url_redacted": self.url_redacted,
            "detail": self.detail,
            "checked_urls": self.checked_urls,
        })
    }
}

impl Default for ProxyDiagnostics {
    fn default() -> Self {
        Self::direct("auto", "proxy bootstrap not run", Vec::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyCandidate {
    source: String,
    url: String,
}

impl ProxyCandidate {
    fn new(source: impl Into<String>, raw_url: impl AsRef<str>) -> Option<Self> {
        let url = normalize_proxy_url(raw_url.as_ref())?;
        Url::parse(&url).ok()?;
        Some(Self {
            source: source.into(),
            url,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn redacted_url(&self) -> String {
        redact_proxy_url(&self.url)
    }
}

pub fn discover_all_candidates() -> Vec<ProxyCandidate> {
    let urls = vec![
        "https://ieeexplore.ieee.org".to_string(),
        "https://dl.acm.org".to_string(),
    ];
    discover_candidates_for_urls(&urls)
}

pub async fn bootstrap(config: &Config) -> GrokResult<(Client, ProxyDiagnostics)> {
    let checked_urls = probe_urls(config);
    let timeout = probe_timeout(config.timeout);
    let mode = config.proxy.trim();
    if mode.eq_ignore_ascii_case("off") {
        return Ok((
            build_client_direct(config.timeout)?,
            ProxyDiagnostics::direct("off", "proxy disabled by configuration", checked_urls),
        ));
    }
    if checked_urls.is_empty() {
        return Ok((
            build_client_direct(config.timeout)?,
            ProxyDiagnostics::direct(mode_label(mode), "skipped/no probeable api", checked_urls),
        ));
    }

    let candidates = if mode.is_empty() || mode.eq_ignore_ascii_case("auto") {
        discover_candidates_for_urls(&checked_urls)
    } else if let Some(candidate) = ProxyCandidate::new("manual", mode) {
        vec![candidate]
    } else {
        return Ok((
            build_client_direct(config.timeout)?,
            ProxyDiagnostics::direct(
                "manual",
                format!("invalid proxy URL: {}", redact_proxy_url(mode)),
                checked_urls,
            ),
        ));
    };

    if candidates.is_empty() {
        return Ok((
            build_client_direct(config.timeout)?,
            ProxyDiagnostics::direct(
                mode_label(mode),
                "no proxy candidate discovered",
                checked_urls,
            ),
        ));
    }

    let mut failures = Vec::new();
    for candidate in candidates {
        let redacted = redact_proxy_url(&candidate.url);
        let client = match build_client_with_proxy(config.timeout, &candidate.url) {
            Ok(client) => client,
            Err(err) => {
                failures.push(format!(
                    "{} {redacted}: {}",
                    candidate.source,
                    sanitize_proxy_error(&err.to_string(), &candidate.url, &redacted)
                ));
                continue;
            }
        };
        match probe_all(&client, &checked_urls, timeout).await {
            Ok(()) => {
                return Ok((
                    client,
                    ProxyDiagnostics {
                        mode: mode_label(mode),
                        status: "proxied".to_string(),
                        source: candidate.source,
                        url_redacted: Some(redacted),
                        detail: "proxy path reached all probeable APIs".to_string(),
                        checked_urls,
                    },
                ));
            }
            Err(err) => failures.push(format!(
                "{} {redacted}: {}",
                candidate.source,
                sanitize_proxy_error(&err, &candidate.url, &redacted)
            )),
        }
    }

    Ok((
        build_client_direct(config.timeout)?,
        ProxyDiagnostics::direct(
            mode_label(mode),
            format!(
                "proxy candidates failed; using direct ({})",
                failures.join("; ")
            ),
            checked_urls,
        ),
    ))
}

fn mode_label(mode: &str) -> String {
    if mode.is_empty() || mode.eq_ignore_ascii_case("auto") {
        "auto".to_string()
    } else {
        "manual".to_string()
    }
}

fn probe_timeout(config_timeout: Duration) -> Duration {
    config_timeout
        .min(Duration::from_secs(8))
        .max(Duration::from_secs(2))
}

fn probe_urls(config: &Config) -> Vec<String> {
    let mut urls = Vec::new();
    match config.transport {
        Transport::Responses => {
            if config.grok_auth_mode == AuthMode::OAuth || config.grok_api_key.is_some() {
                urls.push(config.grok_api_url.clone());
            }
        }
        Transport::ChatCompletions => {
            if config.openai_compatible_api_key.is_some() {
                if let Some(url) = &config.openai_compatible_api_url {
                    urls.push(url.clone());
                }
            }
        }
    }
    if config.tavily_enabled && config.tavily_api_key.is_some() {
        urls.push(config.tavily_api_url.clone());
    }
    if config.firecrawl_enabled && config.firecrawl_api_key.is_some() {
        urls.push(config.firecrawl_api_url.clone());
    }
    dedupe(urls)
}

async fn probe_all(client: &Client, urls: &[String], timeout: Duration) -> Result<(), String> {
    for url in urls {
        probe_one(client, url, timeout)
            .await
            .map_err(|err| format!("{url}: {err}"))?;
    }
    Ok(())
}

async fn probe_one(client: &Client, url: &str, timeout: Duration) -> Result<(), String> {
    let head = tokio::time::timeout(timeout, client.head(url).send()).await;
    match head {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(head_err)) => {
            let get = tokio::time::timeout(timeout, client.get(url).send()).await;
            match get {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(get_err)) => {
                    Err(format!("HEAD failed ({head_err}); GET failed ({get_err})"))
                }
                Err(_) => Err(format!("HEAD failed ({head_err}); GET timed out")),
            }
        }
        Err(_) => {
            let get = tokio::time::timeout(timeout, client.get(url).send()).await;
            match get {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(get_err)) => Err(format!("HEAD timed out; GET failed ({get_err})")),
                Err(_) => Err("HEAD timed out; GET timed out".to_string()),
            }
        }
    }
}

fn discover_candidates_for_urls(urls: &[String]) -> Vec<ProxyCandidate> {
    let env: HashMap<String, String> = std::env::vars().collect();
    let no_proxy = env
        .get("NO_PROXY")
        .or_else(|| env.get("no_proxy"))
        .map(String::as_str)
        .unwrap_or_default();
    let mut candidates = discover_env_candidates(&env, no_proxy, urls);
    candidates.extend(discover_system_candidates());
    dedupe_candidates(candidates)
}

pub fn discover_env_candidates(
    env: &HashMap<String, String>,
    no_proxy: &str,
    urls: &[String],
) -> Vec<ProxyCandidate> {
    if !no_proxy.trim().is_empty() && urls.iter().any(|url| no_proxy_matches(no_proxy, url)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        if let Some(value) = env.get(key).filter(|value| !value.trim().is_empty()) {
            if let Some(candidate) = ProxyCandidate::new(format!("env:{key}"), value) {
                out.push(candidate);
            }
        }
    }
    dedupe_candidates(out)
}

fn discover_system_candidates() -> Vec<ProxyCandidate> {
    #[cfg(target_os = "windows")]
    {
        return discover_windows_candidates();
    }
    #[cfg(target_os = "macos")]
    {
        return discover_macos_candidates();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return discover_linux_candidates();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(target_os = "windows")]
fn discover_windows_candidates() -> Vec<ProxyCandidate> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyEnable",
        ])
        .output()
        .ok();
    let enabled = output
        .as_ref()
        .and_then(|out| String::from_utf8(out.stdout.clone()).ok())
        .map(|text| text.contains("0x1"))
        .unwrap_or(false);
    if !enabled {
        return Vec::new();
    }
    let output = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyServer",
        ])
        .output()
        .ok();
    let Some(text) = output.and_then(|out| String::from_utf8(out.stdout).ok()) else {
        return Vec::new();
    };
    let Some(value) = text.split_whitespace().last() else {
        return Vec::new();
    };
    parse_windows_proxy_server(value)
}

#[cfg(target_os = "macos")]
fn discover_macos_candidates() -> Vec<ProxyCandidate> {
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok();
    let Some(text) = output.and_then(|out| String::from_utf8(out.stdout).ok()) else {
        return Vec::new();
    };
    parse_macos_scutil_proxy(&text)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn discover_linux_candidates() -> Vec<ProxyCandidate> {
    let mut out = Vec::new();
    if let Some(text) = run_command("gsettings", &["get", "org.gnome.system.proxy", "mode"]) {
        out.extend(parse_gnome_proxy_settings(&text, |schema, key| {
            run_command("gsettings", &["get", schema, key])
        }));
    }
    if out.is_empty() {
        if let Some(home) = std::env::var_os("HOME") {
            let path = std::path::Path::new(&home)
                .join(".config")
                .join("kioslaverc");
            if let Ok(text) = std::fs::read_to_string(path) {
                out.extend(parse_kde_kioslaverc(&text));
            }
        }
    }
    out
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    String::from_utf8(output.stdout).ok()
}

pub fn parse_windows_proxy_server(value: &str) -> Vec<ProxyCandidate> {
    let mut out = Vec::new();
    if value.contains('=') {
        for part in value.split(';') {
            let Some((scheme, endpoint)) = part.split_once('=') else {
                continue;
            };
            let scheme = match scheme.trim().to_ascii_lowercase().as_str() {
                "http" | "https" => "http",
                "socks" | "socks5" => "socks5",
                _ => continue,
            };
            if let Some(candidate) =
                ProxyCandidate::new("system:windows", format!("{scheme}://{}", endpoint.trim()))
            {
                out.push(candidate);
            }
        }
    } else if let Some(candidate) = ProxyCandidate::new("system:windows", value) {
        out.push(candidate);
    }
    dedupe_candidates(out)
}

pub fn parse_macos_scutil_proxy(text: &str) -> Vec<ProxyCandidate> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    let mut out = Vec::new();
    for (enabled_key, host_key, port_key, scheme) in [
        ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5"),
    ] {
        if map.get(enabled_key).map(String::as_str) != Some("1") {
            continue;
        }
        let Some(host) = map.get(host_key).filter(|host| !host.is_empty()) else {
            continue;
        };
        let Some(port) = map.get(port_key).filter(|port| !port.is_empty()) else {
            continue;
        };
        if let Some(candidate) =
            ProxyCandidate::new("system:macos", format!("{scheme}://{host}:{port}"))
        {
            out.push(candidate);
        }
    }
    dedupe_candidates(out)
}

pub fn parse_gnome_proxy_settings<F>(mode_output: &str, mut get_value: F) -> Vec<ProxyCandidate>
where
    F: FnMut(&str, &str) -> Option<String>,
{
    if clean_gsettings_value(mode_output) != "manual" {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (schema, scheme) in [
        ("org.gnome.system.proxy.https", "http"),
        ("org.gnome.system.proxy.http", "http"),
        ("org.gnome.system.proxy.socks", "socks5"),
    ] {
        let host = get_value(schema, "host")
            .map(|value| clean_gsettings_value(&value))
            .unwrap_or_default();
        let port = get_value(schema, "port")
            .map(|value| clean_gsettings_value(&value))
            .unwrap_or_default();
        if host.is_empty() || port.is_empty() || port == "0" {
            continue;
        }
        if let Some(candidate) =
            ProxyCandidate::new("system:gnome", format!("{scheme}://{host}:{port}"))
        {
            out.push(candidate);
        }
    }
    dedupe_candidates(out)
}

pub fn parse_kde_kioslaverc(text: &str) -> Vec<ProxyCandidate> {
    let mut in_proxy = false;
    let mut proxy_type = None;
    let mut http_proxy = None;
    let mut https_proxy = None;
    let mut socks_proxy = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_proxy = line == "[Proxy Settings]";
            continue;
        }
        if !in_proxy {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "ProxyType" => proxy_type = Some(value.trim().to_string()),
            "httpProxy" => http_proxy = Some(value.trim().to_string()),
            "httpsProxy" => https_proxy = Some(value.trim().to_string()),
            "socksProxy" => socks_proxy = Some(value.trim().to_string()),
            _ => {}
        }
    }
    if proxy_type.as_deref() != Some("1") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw in [https_proxy, http_proxy, socks_proxy].into_iter().flatten() {
        if let Some(candidate) = ProxyCandidate::new("system:kde", raw) {
            out.push(candidate);
        }
    }
    dedupe_candidates(out)
}

pub fn no_proxy_matches(no_proxy: &str, url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    for raw in no_proxy
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if raw == "*" {
            return true;
        }
        let entry = raw
            .trim_start_matches('.')
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if host == entry || host.ends_with(&format!(".{entry}")) {
            return true;
        }
    }
    false
}

fn normalize_proxy_url(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains("://") {
        return Some(value.to_string());
    }
    Some(format!("http://{value}"))
}

pub fn redact_proxy_url(raw: &str) -> String {
    let Some(normalized) = normalize_proxy_url(raw) else {
        return raw.to_string();
    };
    let Ok(mut url) = Url::parse(&normalized) else {
        return redact_unparseable_proxy(raw);
    };
    if !url.username().is_empty() {
        let _ = url.set_username("***");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }
    let text = url.to_string();
    if raw.contains("://") {
        text
    } else {
        text.trim_start_matches("http://").to_string()
    }
}

fn redact_unparseable_proxy(raw: &str) -> String {
    if let Some((prefix, rest)) = raw.split_once("://") {
        if rest.contains('@') {
            return format!("{prefix}://***");
        }
    }
    if raw.contains('@') {
        return "***".to_string();
    }
    raw.to_string()
}

fn sanitize_proxy_error(detail: &str, raw_url: &str, redacted_url: &str) -> String {
    let mut out = detail.replace(raw_url, redacted_url);
    if let Ok(url) = Url::parse(raw_url) {
        if !url.username().is_empty() {
            out = out.replace(url.username(), "***");
        }
        if let Some(password) = url.password() {
            out = out.replace(password, "***");
        }
    }
    out
}

fn clean_gsettings_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn dedupe_candidates(values: Vec<ProxyCandidate>) -> Vec<ProxyCandidate> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|candidate| seen.insert(candidate.url.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn redacts_proxy_credentials() {
        assert_eq!(
            redact_proxy_url("http://user:pass@127.0.0.1:7890"),
            "http://***:***@127.0.0.1:7890/"
        );
    }

    #[test]
    fn env_candidates_respect_no_proxy() {
        let env = HashMap::from([
            (
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:7890".to_string(),
            ),
            (
                "HTTP_PROXY".to_string(),
                "http://127.0.0.1:8080".to_string(),
            ),
        ]);
        let urls = vec!["https://api.x.ai/v1".to_string()];
        assert!(discover_env_candidates(&env, "api.x.ai", &urls).is_empty());
    }

    #[test]
    fn env_candidates_prefer_https_http_all_order() {
        let env = HashMap::from([
            ("ALL_PROXY".to_string(), "http://127.0.0.1:1000".to_string()),
            (
                "HTTP_PROXY".to_string(),
                "http://127.0.0.1:2000".to_string(),
            ),
            (
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:3000".to_string(),
            ),
        ]);
        let urls = vec!["https://api.x.ai/v1".to_string()];
        let candidates = discover_env_candidates(&env, "", &urls);
        assert_eq!(candidates[0].url, "http://127.0.0.1:3000");
        assert_eq!(candidates[1].url, "http://127.0.0.1:2000");
        assert_eq!(candidates[2].url, "http://127.0.0.1:1000");
    }

    #[test]
    fn parses_windows_proxy_server_forms() {
        let simple = parse_windows_proxy_server("127.0.0.1:7890");
        assert_eq!(simple[0].url, "http://127.0.0.1:7890");

        let split = parse_windows_proxy_server(
            "http=127.0.0.1:7890;https=127.0.0.1:7891;socks=127.0.0.1:1080",
        );
        assert_eq!(split[0].url, "http://127.0.0.1:7890");
        assert_eq!(split[1].url, "http://127.0.0.1:7891");
        assert_eq!(split[2].url, "socks5://127.0.0.1:1080");
    }

    #[test]
    fn parses_macos_scutil_proxy() {
        let candidates = parse_macos_scutil_proxy(
            r#"
<dictionary> {
  HTTPEnable : 1
  HTTPProxy : 127.0.0.1
  HTTPPort : 7890
  HTTPSEnable : 1
  HTTPSProxy : 127.0.0.1
  HTTPSPort : 7891
  SOCKSEnable : 1
  SOCKSProxy : 127.0.0.1
  SOCKSPort : 1080
}
"#,
        );
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].url, "http://127.0.0.1:7890");
        assert_eq!(candidates[1].url, "http://127.0.0.1:7891");
        assert_eq!(candidates[2].url, "socks5://127.0.0.1:1080");
    }

    #[test]
    fn parses_gnome_proxy_settings() {
        let values = HashMap::from([
            (("org.gnome.system.proxy.https", "host"), "'127.0.0.1'"),
            (("org.gnome.system.proxy.https", "port"), "7891"),
            (("org.gnome.system.proxy.http", "host"), "'127.0.0.1'"),
            (("org.gnome.system.proxy.http", "port"), "7890"),
        ]);
        let candidates = parse_gnome_proxy_settings("'manual'", |schema, key| {
            values.get(&(schema, key)).map(|value| value.to_string())
        });
        assert_eq!(candidates[0].url, "http://127.0.0.1:7891");
        assert_eq!(candidates[1].url, "http://127.0.0.1:7890");
    }

    #[test]
    fn parses_kde_kioslaverc() {
        let candidates = parse_kde_kioslaverc(
            r#"
[Proxy Settings]
ProxyType=1
httpProxy=http://127.0.0.1:7890
httpsProxy=http://127.0.0.1:7891
socksProxy=socks5://127.0.0.1:1080
"#,
        );
        assert_eq!(candidates[0].url, "http://127.0.0.1:7891");
        assert_eq!(candidates[1].url, "http://127.0.0.1:7890");
        assert_eq!(candidates[2].url, "socks5://127.0.0.1:1080");
    }

    fn spawn_proxy_probe_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy probe server");
        let addr = listener.local_addr().expect("local addr");
        thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response =
                b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(response);
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn bootstrap_adopts_manual_proxy_when_probe_succeeds() {
        let proxy = spawn_proxy_probe_server();
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GROK_SEARCH_URL", "http://api.example"),
            ("GROK_SEARCH_PROXY", proxy.as_str()),
        ]);

        let (_client, diagnostics) = bootstrap(&config).await.expect("bootstrap");

        assert_eq!(diagnostics.mode, "manual");
        assert_eq!(diagnostics.status, "proxied");
        assert_eq!(diagnostics.source, "manual");
        assert_eq!(
            diagnostics.checked_urls,
            vec!["http://api.example/v1".to_string()]
        );
    }

    #[tokio::test]
    async fn bootstrap_skips_when_no_api_can_be_probed() {
        let config = Config::from_env_map([("GROK_SEARCH_PROXY", "auto")]);

        let (_client, diagnostics) = bootstrap(&config).await.expect("bootstrap");

        assert_eq!(diagnostics.status, "direct");
        assert_eq!(diagnostics.detail, "skipped/no probeable api");
        assert!(diagnostics.checked_urls.is_empty());
    }

    #[tokio::test]
    async fn bootstrap_rejects_invalid_manual_proxy_without_leaking_credentials() {
        let config = Config::from_env_map([
            ("GROK_SEARCH_API_KEY", "xai-fake"),
            ("GROK_SEARCH_PROXY", "http://user:secret@"),
        ]);

        let (_client, diagnostics) = bootstrap(&config).await.expect("bootstrap");

        assert_eq!(diagnostics.status, "direct");
        assert!(diagnostics.detail.contains("invalid proxy URL"));
        assert!(!diagnostics.detail.contains("user:secret"));
        assert!(!diagnostics.detail.contains("secret@"));
    }
}
