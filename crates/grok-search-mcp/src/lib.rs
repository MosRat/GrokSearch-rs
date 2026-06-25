use std::sync::Arc;

use grok_search_service::SearchService;
use grok_search_types::model::tool::WebSearchInput;
use grok_search_types::{GrokSearchError, Result};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::schemars::JsonSchema;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Value};

pub async fn run_stdio(service: SearchService) -> anyhow::Result<()> {
    let server = McpServer { service }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct McpServer {
    service: SearchService,
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "grok-search-rs".to_string(),
                title: None,
                version: env!("CARGO_PKG_VERSION").to_string(),
                icons: None,
                website_url: None,
            },
            instructions: None,
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: tools(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        let value = call_tool(&self.service, request.name.as_ref(), Value::Object(args))
            .await
            .map_err(mcp_error)?;
        Ok(CallToolResult {
            content: vec![Content::text(value.to_string())],
            structured_content: Some(value),
            is_error: Some(false),
            meta: None,
        })
    }
}

async fn call_tool(service: &SearchService, name: &str, args: Value) -> Result<Value> {
    match name {
        "doctor" => Ok(service.doctor().await),
        "web_search" => {
            let params: WebSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.query.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "web_search.query is required".into(),
                ));
            }
            let output = service.web_search(params.into()).await?;
            serialize_output(output, "serialize output")
        }
        "get_sources" => {
            let params: GetSourcesParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .get_sources(
                    &params.session_id,
                    params.offset.unwrap_or(0),
                    params.limit.filter(|value| *value > 0),
                )
                .await?;
            serialize_output(output, "serialize sources")
        }
        "web_fetch" => {
            let params: WebFetchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .web_fetch(&params.url, params.max_chars.filter(|value| *value > 0))
                .await?;
            serialize_output(output, "serialize fetch")
        }
        "web_map" => {
            let params: WebMapParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let max_results = params.max_results.unwrap_or(10);
            let sources = service.web_map(&params.url, max_results).await?;
            let mapped_sources: Vec<Value> = sources
                .iter()
                .map(|source| json!({ "url": &source.url, "provider": &source.provider }))
                .collect();
            Ok(json!({
                "url": params.url,
                "sources_count": mapped_sources.len(),
                "sources": mapped_sources
            }))
        }
        _ => Err(GrokSearchError::NotFound(format!("unknown tool: {name}"))),
    }
}

fn serialize_output<T: serde::Serialize>(output: T, context: &str) -> Result<Value> {
    serde_json::to_value(output).map_err(|err| GrokSearchError::Parse(format!("{context}: {err}")))
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebSearchParams {
    query: String,
    platform: Option<String>,
    model: Option<String>,
    extra_sources: Option<usize>,
    recency_days: Option<u32>,
    #[serde(default)]
    include_domains: Vec<String>,
    #[serde(default)]
    exclude_domains: Vec<String>,
    include_content: Option<bool>,
    response_format: Option<String>,
}

impl From<WebSearchParams> for WebSearchInput {
    fn from(params: WebSearchParams) -> Self {
        Self {
            query: params.query,
            platform: params.platform,
            model: params.model,
            extra_sources: params.extra_sources,
            recency_days: params.recency_days.filter(|value| *value > 0),
            include_domains: params.include_domains,
            exclude_domains: params.exclude_domains,
            include_content: params.include_content,
            response_format: params.response_format,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetSourcesParams {
    session_id: String,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebFetchParams {
    url: String,
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebMapParams {
    url: String,
    max_results: Option<usize>,
}

fn tools() -> Vec<Tool> {
    tools_list()["tools"]
        .as_array()
        .expect("tools_list is an array")
        .iter()
        .map(tool_from_value)
        .collect()
}

fn tool_from_value(value: &Value) -> Tool {
    let name = value["name"].as_str().expect("tool name");
    let description = value["description"].as_str().expect("tool description");
    let input_schema = value["inputSchema"]
        .as_object()
        .expect("input schema")
        .clone();
    Tool {
        name: name.to_string().into(),
        title: None,
        description: Some(description.to_string().into()),
        input_schema: Arc::new(input_schema),
        output_schema: None,
        annotations: None,
        icons: None,
    }
}

fn mcp_error(error: GrokSearchError) -> McpError {
    let message = error.to_string();
    let data = Some(json!({ "code": error.code() }));
    match error {
        GrokSearchError::InvalidParams(_) => McpError::invalid_params(message, data),
        GrokSearchError::NotFound(_) => McpError::resource_not_found(message, data),
        GrokSearchError::Parse(_) => McpError::parse_error(message, data),
        _ => McpError::internal_error(message, data),
    }
}

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "web_search",
                "description": "Use for discovery — when you don't have a specific URL and need to find information, debug an error, research a topic, or track down an issue or news item. Returns an AI-synthesised answer plus a source list. By default the first few sources carry inline content (max_inline_sources, default 5); the rest are metadata-only — drill into any of them with web_fetch(url). The whole response is capped by a character budget; when truncated=true, trimmed sources carry a note telling you how to recover the full text via web_fetch or get_sources. Pass response_format=\"concise\" for answer + source metadata only. If you already know the exact page URL, use web_fetch instead.",
                "inputSchema": {
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "platform": { "type": "string" },
                        "model": { "type": "string" },
                        "extra_sources": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Optional supplemental source count. Tavily is primary; Firecrawl is fallback. If omitted, GROK_SEARCH_EXTRA_SOURCES is used."
                        },
                        "recency_days": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Restrict supplemental results to sources published within the last N days. Forwarded to Tavily as days+topic=news; also hinted to Grok prompt."
                        },
                        "include_domains": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Only return supplemental results from these domains. Tavily honors strictly; Grok receives as soft preference."
                        },
                        "exclude_domains": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Suppress supplemental results from these domains. Tavily honors strictly; Grok receives as soft instruction."
                        },
                        "include_content": {
                            "type": "boolean",
                            "default": true,
                            "description": "Inline source content via the resolve_content pipeline. Default true. Pass false to get summary + source-list only (legacy behavior, no content field in sources). Superseded by response_format when both are set."
                        },
                        "response_format": {
                            "type": "string",
                            "enum": ["concise", "detailed"],
                            "description": "concise = synthesized answer + source metadata only (smallest payload); detailed = inline source content, subject to the response budget. Takes precedence over include_content."
                        }
                    }
                }
            },
            {
                "name": "get_sources",
                "description": "Return cached sources from a previous web_search call by session_id. Use to re-examine sources already retrieved without issuing a new search — it reuses the prior session and runs no new search or fetch. Paginate with offset/limit: the response reports total_sources and, when more pages remain, next_offset to pass as the next offset.",
                "inputSchema": {
                    "type": "object",
                    "required": ["session_id"],
                    "properties": {
                        "session_id": { "type": "string" },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 0,
                            "description": "Index of the first source to return. Use next_offset from the previous page to continue."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Max sources in this page. Omit to return all remaining sources (still subject to the response budget)."
                        }
                    }
                }
            },
            {
                "name": "web_fetch",
                "description": "Use when you already have a specific URL and want to read a single page in depth. GitHub issue/PR, StackOverflow (StackExchange), arXiv, and Wikipedia URLs are automatically parsed into structured, de-noised Markdown ready to feed an LLM; all other pages fall back to generic extraction. Returns {url, content, original_length, truncated, source_type, fallback_reason?}. If you don't have a URL yet and need to discover sources, use web_search instead.",
                "inputSchema": {
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": { "type": "string" },
                        "max_chars": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional character cap on returned content. Falls back to GROK_SEARCH_FETCH_MAX_CHARS, otherwise unlimited."
                        }
                    }
                }
            },
            {
                "name": "web_map",
                "description": "Map/discover URLs through Tavily Map.",
                "inputSchema": {
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": { "type": "string" },
                        "max_results": { "type": "integer", "minimum": 1 }
                    }
                }
            },
            {
                "name": "doctor",
                "description": "Diagnostic probe: live connectivity check for Grok, Tavily, and Firecrawl backends, plus masked configuration. Use to verify the server is wired up and reachable.",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_contains_existing_tools() {
        let names: Vec<_> = tools().into_iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            vec![
                "web_search",
                "get_sources",
                "web_fetch",
                "web_map",
                "doctor"
            ]
        );
    }

    #[test]
    fn typed_web_search_args_accept_existing_shape() {
        let params: WebSearchParams = serde_json::from_value(json!({
            "query": "rust mcp",
            "platform": "x",
            "model": "grok-4",
            "extra_sources": 2,
            "recency_days": 7,
            "include_domains": ["example.com"],
            "exclude_domains": ["old.example"],
            "include_content": true,
            "response_format": "detailed"
        }))
        .expect("valid params");

        let input: WebSearchInput = params.into();
        assert_eq!(input.query, "rust mcp");
        assert_eq!(input.extra_sources, Some(2));
        assert_eq!(input.recency_days, Some(7));
        assert_eq!(input.include_domains, vec!["example.com"]);
    }

    #[test]
    fn tools_list_descriptions_guide_routing() {
        let listed = tools_list();
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
}
