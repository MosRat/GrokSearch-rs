use grok_search_service::SearchService;
use grok_search_tools::invoke_tool;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServerHandler};
use serde_json::Value;

use crate::convert::{mcp_error, tool_from_spec};

#[derive(Clone)]
pub(crate) struct McpServer {
    service: SearchService,
}

impl McpServer {
    pub(crate) fn new(service: SearchService) -> Self {
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
