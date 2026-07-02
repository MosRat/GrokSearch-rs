use crate::convert::mcp_error;
use crate::http::{build_http_router, McpHttpOptions};
use grok_search_service::SearchService;
use grok_search_types::GrokSearchError;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc as StdArc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration as TokioDuration};
use tokio_util::sync::CancellationToken;

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
    let err = mcp_error(GrokSearchError::SecurityPolicy(
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
    let options = McpHttpOptions::new("127.0.0.1:0".parse().unwrap(), "/mcp/", None, None).unwrap();
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
async fn http_audit_endpoints_share_service_state_and_auth() {
    let (base_url, shutdown, task) = spawn_test_http(Some("secret".to_string())).await;
    let client = reqwest::Client::new();

    let unauthorized = client
        .get(format!("{base_url}/audit"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    for (method, path) in [
        ("GET", "/audit"),
        ("GET", "/audit/recent"),
        ("POST", "/audit/clear"),
    ] {
        let response = match method {
            "GET" => client.get(format!("{base_url}{path}")),
            "POST" => client.post(format!("{base_url}{path}")),
            _ => unreachable!(),
        }
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{method} {path} should reject wrong token"
        );
    }

    let session_id = initialize_authorized_session(&client, &base_url, "secret").await;
    let search = call_authorized_tool(
        &client,
        &base_url,
        &session_id,
        "secret",
        "audit-search".to_string(),
        "web_search",
        json!({ "query": "audit endpoint", "extra_sources": 1 }),
    )
    .await;
    assert!(search["result"]["structuredContent"]["session_id"].is_string());

    let audit: serde_json::Value = client
        .get(format!("{base_url}/audit"))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit["summary"]["tools"]["web_search"]["success"], 1);
    assert_eq!(audit["recent"][0]["tool_name"], "web_search");

    let recent: serde_json::Value = client
        .get(format!(
            "{base_url}/audit/recent?limit=1&tool=web_search&status=success"
        ))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recent.as_array().expect("recent array").len(), 1);

    let invalid_status = client
        .get(format!("{base_url}/audit/recent?status=bogus"))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_status.status(), reqwest::StatusCode::BAD_REQUEST);

    let cleared: serde_json::Value = client
        .post(format!("{base_url}/audit/clear"))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared["ok"], true);

    let audit: serde_json::Value = client
        .get(format!("{base_url}/audit"))
        .bearer_auth("secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        audit["summary"]["tools"]
            .as_object()
            .expect("summary tools object")
            .len(),
        0
    );

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
                    let session_id = initialize_session(&client, &base_url, agent_id, round).await;
                    let listed = list_tools(&client, &base_url, &session_id, agent_id, round).await;
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

async fn initialize_authorized_session(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "init-authorized",
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "audit-test", "version": "0.0.0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "authorized initialize status {}",
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

async fn call_authorized_tool(
    client: &reqwest::Client,
    base_url: &str,
    session_id: &str,
    token: &str,
    id: String,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let body = client
        .post(format!("{base_url}/mcp"))
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", session_id)
        .bearer_auth(token)
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
