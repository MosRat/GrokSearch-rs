use std::sync::Arc;

use grok_search_service::SearchService;
use grok_search_tools::{invoke_tool, ToolSpec};
use grok_search_types::GrokSearchError;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::stdio;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde_json::Value;

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
            tools: grok_search_tools::tools()
                .into_iter()
                .map(tool_from_spec)
                .collect(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        let args = request.arguments.unwrap_or_default();
        let value = invoke_tool(&self.service, request.name.as_ref(), Value::Object(args))
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

fn tool_from_spec(spec: ToolSpec) -> Tool {
    Tool {
        name: spec.name.into(),
        title: None,
        description: Some(spec.description.into()),
        input_schema: Arc::new(spec.input_schema),
        output_schema: None,
        annotations: None,
        icons: None,
    }
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

#[cfg(test)]
mod tests {
    use grok_search_types::GrokSearchError;

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
                "academic_read",
                "academic_parse_pdf",
                "academic_download_pdf"
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
}
