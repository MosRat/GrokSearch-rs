use std::net::{IpAddr, SocketAddr};

use anyhow::Result;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

pub(crate) async fn require_bearer_token(
    request: Request,
    next: Next,
    expected_token: String,
) -> Result<Response, StatusCode> {
    if request.uri().path() == "/healthz" {
        return Ok(next.run(request).await);
    }
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| constant_time_eq(token.as_bytes(), expected_token.as_bytes()));
    if authorized {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub(crate) fn normalize_http_path(path: String) -> anyhow::Result<String> {
    let path = path.trim();
    if path.is_empty() {
        anyhow::bail!("HTTP MCP path cannot be empty");
    }
    if !path.starts_with('/') {
        anyhow::bail!("HTTP MCP path must start with '/'");
    }
    if path.contains('?') || path.contains('#') {
        anyhow::bail!("HTTP MCP path must not include query or fragment");
    }
    if path != "/" && path.ends_with('/') {
        return Ok(path.trim_end_matches('/').to_string());
    }
    Ok(path.to_string())
}

pub(crate) fn validate_origin(origin: &str) -> anyhow::Result<()> {
    let url = url::Url::parse(origin)?;
    if url.scheme() != "http" && url.scheme() != "https" {
        anyhow::bail!("HTTP MCP allow origin must use http or https");
    }
    if url.host_str().is_none() {
        anyhow::bail!("HTTP MCP allow origin must include a host");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("HTTP MCP allow origin must be an origin only, without path/query/fragment");
    }
    Ok(())
}

pub(crate) fn allowed_hosts_for(bind: SocketAddr) -> Vec<String> {
    let ip = bind.ip();
    if ip.is_unspecified() {
        return Vec::new();
    }
    let mut hosts = match ip {
        IpAddr::V4(addr) if addr.is_loopback() => {
            vec!["localhost".to_string(), "127.0.0.1".to_string()]
        }
        IpAddr::V6(addr) if addr.is_loopback() => vec!["localhost".to_string(), "::1".to_string()],
        _ => vec![ip.to_string()],
    };
    if bind.port() != 0 {
        hosts.push(format!("{}:{}", host_for_authority(ip), bind.port()));
    }
    hosts
}

pub(crate) fn is_loopback_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => addr.is_loopback(),
        IpAddr::V6(addr) => addr.is_loopback(),
    }
}

pub(crate) fn nonempty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn host_for_authority(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(addr) => addr.to_string(),
        IpAddr::V6(addr) => format!("[{addr}]"),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}
