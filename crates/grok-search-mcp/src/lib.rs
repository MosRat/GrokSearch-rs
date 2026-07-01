use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use grok_search_service::SearchService;
use grok_search_tools::{invoke_tool, ToolSpec};
use grok_search_types::GrokSearchError;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

const DEFAULT_HTTP_BODY_LIMIT_BYTES: usize = 1024 * 1024;
const DEFAULT_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run_stdio(service: SearchService) -> anyhow::Result<()> {
    let server = McpServer::new(service).serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

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

async fn run_http_with_shutdown(
    service: SearchService,
    options: McpHttpOptions,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(options.bind).await?;
    let local_addr = listener.local_addr()?;
    let cancellation_token = CancellationToken::new();
    let router = build_http_router(service, &options, cancellation_token.clone())?;

    eprintln!(
        "grok-search-rs HTTP MCP listening on http://{}{} (auth: {}, cors: {})",
        local_addr,
        options.path,
        options.auth_status(),
        options.allow_origin.as_deref().unwrap_or("disabled")
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            cancellation_token.cancel();
        })
        .await?;
    Ok(())
}

fn build_http_router(
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
        streamable_http_config(&options, cancellation_token.clone()),
    );

    let mut router = Router::new()
        .route("/healthz", get(healthz))
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

#[derive(Clone)]
struct McpServer {
    service: SearchService,
}

impl McpServer {
    fn new(service: SearchService) -> Self {
        Self { service }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("grok-search-rs", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: grok_search_tools::tools()
                .into_iter()
                .map(tool_from_spec)
                .collect(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        let value = invoke_tool(&self.service, request.name.as_ref(), Value::Object(args))
            .await
            .map_err(mcp_error)?;
        Ok(CallToolResult::structured(value))
    }
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

async fn require_bearer_token(
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

fn tool_from_spec(spec: ToolSpec) -> Tool {
    Tool::new(spec.name, spec.description, spec.input_schema)
}

fn mcp_error(error: GrokSearchError) -> McpError {
    let message = error.to_string();
    let data = Some(error.diagnostics());
    match error {
        GrokSearchError::InvalidParams(_) => McpError::invalid_params(message, data),
        GrokSearchError::SecurityPolicy(_) => McpError::invalid_params(message, data),
        GrokSearchError::NotFound(_) => McpError::resource_not_found(message, data),
        GrokSearchError::Parse(_) => McpError::parse_error(message, data),
        _ => McpError::internal_error(message, data),
    }
}

fn normalize_http_path(path: String) -> anyhow::Result<String> {
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

fn validate_origin(origin: &str) -> anyhow::Result<()> {
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

fn allowed_hosts_for(bind: SocketAddr) -> Vec<String> {
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

fn host_for_authority(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(addr) => addr.to_string(),
        IpAddr::V6(addr) => format!("[{addr}]"),
    }
}

fn is_loopback_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(addr) => addr.is_loopback(),
        IpAddr::V6(addr) => addr.is_loopback(),
    }
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use grok_search_types::GrokSearchError;
    use std::collections::HashSet;
    use std::sync::Arc as StdArc;
    use tokio::task::JoinHandle;
    use tokio::time::{timeout, Duration as TokioDuration};

    #[test]
    fn tools_list_contains_existing_tools() {
        let names: Vec<_> = grok_search_tools::tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "web_search",
                "get_sources",
                "web_fetch",
                "web_map",
                "wechat_search",
                "zhihu_search",
                "doctor",
                "repo_metadata",
                "academic_search",
                "academic_get",
                "academic_citations",
                "academic_pdf_read",
                "academic_pdf_structure",
                "academic_pdf_artifacts",
                "academic_pdf_download"
            ]
        );
    }

    #[test]
    fn tools_list_descriptions_guide_routing() {
        let listed = grok_search_tools::tools_list_json();
        let tools = listed["tools"].as_array().expect("tools array");

        let desc = |name: &str| -> String {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("tool {name} missing"))["description"]
                .as_str()
                .unwrap_or_else(|| panic!("tool {name} description not a string"))
                .to_string()
        };

        let web_search = desc("web_search");
        assert!(web_search.contains("discovery"), "web_search: {web_search}");
        assert!(
            web_search.contains("don't have a specific URL"),
            "web_search: {web_search}"
        );
        assert!(
            !web_search.contains("read a single page"),
            "web_search must not claim the single-page-read role: {web_search}"
        );

        let web_fetch = desc("web_fetch");
        assert!(web_fetch.contains("specific URL"), "web_fetch: {web_fetch}");
        assert!(
            web_fetch.contains("read a single page"),
            "web_fetch: {web_fetch}"
        );
        assert!(web_fetch.contains("GitHub issue"), "web_fetch: {web_fetch}");
        assert!(
            web_fetch.contains("StackOverflow") || web_fetch.contains("StackExchange"),
            "web_fetch: {web_fetch}"
        );
        assert!(web_fetch.contains("arXiv"), "web_fetch: {web_fetch}");
        assert!(web_fetch.contains("Wikipedia"), "web_fetch: {web_fetch}");
        assert!(web_fetch.contains("web_search"), "web_fetch: {web_fetch}");

        let get_sources = desc("get_sources");
        assert!(
            get_sources.contains("session_id"),
            "get_sources: {get_sources}"
        );
        assert!(
            get_sources.contains("new search"),
            "get_sources: {get_sources}"
        );
    }

    #[test]
    fn mcp_error_data_contains_structured_diagnostics() {
        let err = super::mcp_error(GrokSearchError::SecurityPolicy(
            "url must resolve to a public http or https address".to_string(),
        ));
        let data = err.data.expect("diagnostic data");
        assert_eq!(data["kind"], "security_policy");
        assert_eq!(data["code"], -32602);
        assert_eq!(data["retryable"], false);
        assert!(data["hint"].as_str().unwrap().contains("public"));
    }

    #[test]
    fn http_options_reject_remote_without_token() {
        let err = McpHttpOptions::new("0.0.0.0:8787".parse().unwrap(), "/mcp", None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("GROK_SEARCH_MCP_HTTP_AUTH_TOKEN"));
    }

    #[test]
    fn http_options_accept_loopback_without_token() {
        let options =
            McpHttpOptions::new("127.0.0.1:0".parse().unwrap(), "/mcp/", None, None).unwrap();
        assert_eq!(options.path, "/mcp");
        assert_eq!(options.auth_status(), "unset");
    }

    #[test]
    fn http_options_validate_path_and_origin() {
        assert!(McpHttpOptions::new("127.0.0.1:0".parse().unwrap(), "mcp", None, None).is_err());
        assert!(McpHttpOptions::new(
            "127.0.0.1:0".parse().unwrap(),
            "/mcp",
            None,
            Some("https://example.com/app".to_string()),
        )
        .is_err());
    }

    #[tokio::test]
    async fn http_server_healthz_and_tools_list_work() {
        let (base_url, shutdown, task) = spawn_test_http(None).await;
        let client = reqwest::Client::new();

        let health: serde_json::Value = client
            .get(format!("{base_url}/healthz"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ok"], true);

        let init = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0.0.0" }
                }
            }))
            .send()
            .await
            .unwrap();
        assert!(
            init.status().is_success(),
            "initialize status {}",
            init.status()
        );
        let session_id = init
            .headers()
            .get("mcp-session-id")
            .expect("session id")
            .to_str()
            .unwrap()
            .to_string();

        let listed_body = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let listed = json_from_streamable_http_body(&listed_body);
        let tools = listed["result"]["tools"].as_array().expect("tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "web_search"));
        assert!(tools.iter().any(|tool| tool["name"] == "academic_pdf_read"));

        shutdown.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn http_server_requires_bearer_token_when_configured() {
        let (base_url, shutdown, task) = spawn_test_http(Some("secret".to_string())).await;
        let client = reqwest::Client::new();

        let unauthorized = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        let authorized = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .bearer_auth("secret")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert_ne!(authorized.status(), reqwest::StatusCode::UNAUTHORIZED);

        shutdown.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn http_server_handles_concurrent_independent_sessions() {
        timeout(TokioDuration::from_secs(10), async {
            let (base_url, shutdown, task) = spawn_test_http(None).await;
            let barrier = StdArc::new(tokio::sync::Barrier::new(8));

            let mut clients = Vec::new();
            for agent_id in 0..8 {
                let base_url = base_url.clone();
                let barrier = barrier.clone();
                clients.push(tokio::spawn(async move {
                    let client = reqwest::Client::new();
                    barrier.wait().await;
                    let mut session_ids = Vec::new();
                    for round in 0..5 {
                        let session_id =
                            initialize_session(&client, &base_url, agent_id, round).await;
                        let listed =
                            list_tools(&client, &base_url, &session_id, agent_id, round).await;
                        let tools = listed["result"]["tools"].as_array().expect("tools array");
                        assert!(
                            tools.iter().any(|tool| tool["name"] == "web_search"),
                            "agent {agent_id} round {round} missing web_search"
                        );
                        assert!(
                            tools
                                .iter()
                                .any(|tool| tool["name"] == "academic_pdf_structure"),
                            "agent {agent_id} round {round} missing academic_pdf_structure"
                        );
                        session_ids.push(session_id);
                    }
                    session_ids
                }));
            }

            let mut all_session_ids = Vec::new();
            for client in clients {
                all_session_ids.extend(client.await.unwrap());
            }
            let unique: HashSet<_> = all_session_ids.iter().collect();
            assert_eq!(
                unique.len(),
                all_session_ids.len(),
                "session ids must be unique"
            );

            shutdown.cancel();
            task.await.unwrap().unwrap();
        })
        .await
        .expect("concurrent HTTP MCP test timed out");
    }

    #[tokio::test]
    async fn http_server_rejects_missing_and_unknown_sessions() {
        timeout(TokioDuration::from_secs(10), async {
            let (base_url, shutdown, task) = spawn_test_http(None).await;
            let client = reqwest::Client::new();

            let missing_session = client
                .post(format!("{base_url}/mcp"))
                .header("accept", "application/json, text/event-stream")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": "missing-session",
                    "method": "tools/list",
                    "params": {}
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(
                missing_session.status(),
                reqwest::StatusCode::UNPROCESSABLE_ENTITY
            );

            let unknown_session = client
                .post(format!("{base_url}/mcp"))
                .header("accept", "application/json, text/event-stream")
                .header("mcp-session-id", "unknown-session")
                .json(&json!({
                    "jsonrpc": "2.0",
                    "id": "unknown-session",
                    "method": "tools/list",
                    "params": {}
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(unknown_session.status(), reqwest::StatusCode::NOT_FOUND);

            shutdown.cancel();
            task.await.unwrap().unwrap();
        })
        .await
        .expect("session rejection test timed out");
    }

    #[tokio::test]
    async fn http_server_handles_concurrent_business_tool_calls() {
        timeout(TokioDuration::from_secs(10), async {
            let (base_url, shutdown, task) = spawn_test_http(None).await;
            let barrier = StdArc::new(tokio::sync::Barrier::new(6));

            let mut clients = Vec::new();
            for agent_id in 0..6 {
                let base_url = base_url.clone();
                let barrier = barrier.clone();
                clients.push(tokio::spawn(async move {
                    let client = reqwest::Client::new();
                    let session_id = initialize_session(&client, &base_url, agent_id, 0).await;
                    barrier.wait().await;

                    let search = call_tool(
                        &client,
                        &base_url,
                        &session_id,
                        format!("search-{agent_id}"),
                        "web_search",
                        json!({
                            "query": format!("agent {agent_id} rust mcp"),
                            "include_content": true,
                            "response_format": "detailed"
                        }),
                    )
                    .await;
                    let structured = &search["result"]["structuredContent"];
                    let search_session_id = structured["session_id"]
                        .as_str()
                        .expect("web_search session_id")
                        .to_string();
                    assert_eq!(structured["sources_count"], 4);
                    assert!(
                        structured["content"]
                            .as_str()
                            .expect("content")
                            .contains("OpenAI published"),
                        "unexpected content: {structured}"
                    );

                    let sources = call_tool(
                        &client,
                        &base_url,
                        &session_id,
                        format!("sources-{agent_id}"),
                        "get_sources",
                        json!({
                            "session_id": search_session_id,
                            "offset": 0,
                            "limit": 2
                        }),
                    )
                    .await;
                    let structured_sources = &sources["result"]["structuredContent"];
                    assert_eq!(structured_sources["sources_count"], 2);
                    assert_eq!(
                        structured_sources["sources"][0]["url"],
                        "https://openai.com/news"
                    );
                }));
            }

            for client in clients {
                client.await.unwrap();
            }

            shutdown.cancel();
            task.await.unwrap().unwrap();
        })
        .await
        .expect("concurrent business tool call test timed out");
    }

    async fn spawn_test_http(
        auth_token: Option<String>,
    ) -> (String, CancellationToken, JoinHandle<anyhow::Result<()>>) {
        let options =
            McpHttpOptions::new("127.0.0.1:0".parse().unwrap(), "/mcp", auth_token, None).unwrap();
        let listener = TcpListener::bind(options.bind).await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();
        let router = build_http_router(
            SearchService::fake_with_sources(),
            &options,
            shutdown.clone(),
        )
        .unwrap();
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    task_shutdown.cancelled().await;
                })
                .await?;
            Ok(())
        });
        (format!("http://{local_addr}"), shutdown, task)
    }

    async fn initialize_session(
        client: &reqwest::Client,
        base_url: &str,
        agent_id: usize,
        round: usize,
    ) -> String {
        let response = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .json(&json!({
                "jsonrpc": "2.0",
                "id": format!("init-{agent_id}-{round}"),
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {
                        "name": format!("agent-{agent_id}"),
                        "version": "0.0.0"
                    }
                }
            }))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "agent {agent_id} round {round} initialize status {}",
            response.status()
        );
        response
            .headers()
            .get("mcp-session-id")
            .expect("session id")
            .to_str()
            .unwrap()
            .to_string()
    }

    async fn list_tools(
        client: &reqwest::Client,
        base_url: &str,
        session_id: &str,
        agent_id: usize,
        round: usize,
    ) -> serde_json::Value {
        let body = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": format!("list-{agent_id}-{round}"),
                "method": "tools/list",
                "params": {}
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        json_from_streamable_http_body(&body)
    }

    async fn call_tool(
        client: &reqwest::Client,
        base_url: &str,
        session_id: &str,
        id: String,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let body = client
            .post(format!("{base_url}/mcp"))
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments
                }
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        json_from_streamable_http_body(&body)
    }

    fn json_from_streamable_http_body(body: &str) -> serde_json::Value {
        if let Ok(value) = serde_json::from_str(body) {
            return value;
        }
        let data = body
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim))
            .find(|data| !data.is_empty())
            .expect("SSE data line");
        serde_json::from_str(data).expect("SSE data JSON")
    }
}
