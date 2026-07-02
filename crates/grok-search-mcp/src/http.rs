use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Query;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware;
use axum::routing::{get, post};
use axum::{Json, Router};
use grok_search_service::SearchService;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::handler::McpServer;
use crate::security::{
    allowed_hosts_for, is_loopback_addr, nonempty, normalize_http_path, require_bearer_token,
    validate_origin,
};

const DEFAULT_HTTP_BODY_LIMIT_BYTES: usize = 1024 * 1024;
const DEFAULT_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHttpOptions {
    pub bind: SocketAddr,
    pub path: String,
    pub auth_token: Option<String>,
    pub allow_origin: Option<String>,
}

impl McpHttpOptions {
    pub fn new(
        bind: SocketAddr,
        path: impl Into<String>,
        auth_token: Option<String>,
        allow_origin: Option<String>,
    ) -> anyhow::Result<Self> {
        let path = normalize_http_path(path.into())?;
        let auth_token = auth_token.and_then(nonempty);
        let allow_origin = allow_origin.and_then(nonempty);
        let options = Self {
            bind,
            path,
            auth_token,
            allow_origin,
        };
        options.validate_security()?;
        Ok(options)
    }

    pub fn auth_status(&self) -> &'static str {
        if self.auth_token.is_some() {
            "set"
        } else {
            "unset"
        }
    }

    fn validate_security(&self) -> anyhow::Result<()> {
        if !is_loopback_addr(self.bind.ip()) && self.auth_token.is_none() {
            anyhow::bail!(
                "HTTP MCP bind address {} is not loopback; set GROK_SEARCH_MCP_HTTP_AUTH_TOKEN before exposing the server",
                self.bind
            );
        }
        if let Some(origin) = &self.allow_origin {
            validate_origin(origin)?;
        }
        Ok(())
    }
}

pub async fn run_http(service: SearchService, options: McpHttpOptions) -> anyhow::Result<()> {
    run_http_with_shutdown(service, options, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

pub(crate) async fn run_http_with_shutdown(
    service: SearchService,
    options: McpHttpOptions,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(options.bind).await?;
    let local_addr = listener.local_addr()?;
    let cancellation_token = CancellationToken::new();
    let router = build_http_router(service, &options, cancellation_token.clone())?;

    tracing::info!(
        target: "grok_search",
        address = %local_addr,
        path = %options.path,
        auth = options.auth_status(),
        cors = options.allow_origin.as_deref().unwrap_or("disabled"),
        "HTTP MCP listening"
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            cancellation_token.cancel();
        })
        .await?;
    Ok(())
}

pub(crate) fn build_http_router(
    service: SearchService,
    options: &McpHttpOptions,
    cancellation_token: CancellationToken,
) -> anyhow::Result<Router> {
    let mcp_service = StreamableHttpService::new(
        {
            let service = service.clone();
            move || Ok(McpServer::new(service.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        streamable_http_config(options, cancellation_token.clone()),
    );

    let mut router = Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/audit",
            get({
                let service = service.clone();
                move || async move {
                    Json(
                        serde_json::to_value(service.audit_snapshot(Default::default()))
                            .unwrap_or_else(|_| json!({ "error": "audit_snapshot_failed" })),
                    )
                }
            }),
        )
        .route(
            "/audit/recent",
            get({
                let service = service.clone();
                move |Query(query): Query<AuditRecentHttpQuery>| async move {
                    let query = match query.try_into_audit_query() {
                        Ok(query) => query,
                        Err(err) => {
                            return Err((
                                StatusCode::BAD_REQUEST,
                                Json(json!({ "error": "invalid_status", "message": err })),
                            ));
                        }
                    };
                    Ok(Json(
                        serde_json::to_value(service.audit_recent(query))
                            .unwrap_or_else(|_| json!({ "error": "audit_recent_failed" })),
                    ))
                }
            }),
        )
        .route(
            "/audit/clear",
            post({
                let service = service.clone();
                move || async move {
                    match service.audit_clear() {
                        Ok(()) => Json(json!({ "ok": true })),
                        Err(err) => Json(json!({ "ok": false, "error": err.to_string() })),
                    }
                }
            }),
        )
        .nest_service(&options.path, mcp_service)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            DEFAULT_HTTP_REQUEST_TIMEOUT,
        ))
        .layer(RequestBodyLimitLayer::new(DEFAULT_HTTP_BODY_LIMIT_BYTES));

    if let Some(token) = options.auth_token.clone() {
        router = router.layer(middleware::from_fn(move |request, next| {
            require_bearer_token(request, next, token.clone())
        }));
    }

    if let Some(origin) = &options.allow_origin {
        let origin = HeaderValue::from_str(origin)?;
        router = router.layer(CorsLayer::new().allow_origin(AllowOrigin::exact(origin)));
    }

    Ok(router)
}

fn streamable_http_config(
    options: &McpHttpOptions,
    cancellation_token: CancellationToken,
) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default()
        .with_stateful_mode(true)
        .with_cancellation_token(cancellation_token)
        .with_allowed_hosts(allowed_hosts_for(options.bind));
    if let Some(origin) = &options.allow_origin {
        config = config.with_allowed_origins([origin.clone()]);
    }
    config
}

async fn healthz() -> Json<Value> {
    Json(json!({
        "ok": true,
        "server": "grok-search-rs",
        "transport": "streamable_http"
    }))
}

#[derive(Debug, serde::Deserialize)]
struct AuditRecentHttpQuery {
    limit: Option<usize>,
    tool: Option<String>,
    status: Option<String>,
}

impl AuditRecentHttpQuery {
    fn try_into_audit_query(self) -> Result<grok_search_audit::AuditRecentQuery, String> {
        let status = match self.status.as_deref() {
            Some("success") => Some(grok_search_audit::AuditStatus::Success),
            Some("error") => Some(grok_search_audit::AuditStatus::Error),
            Some(other) => return Err(format!("status must be success or error, got {other}")),
            None => None,
        };
        Ok(grok_search_audit::AuditRecentQuery {
            limit: self.limit,
            tool: self.tool,
            status,
        })
    }
}
