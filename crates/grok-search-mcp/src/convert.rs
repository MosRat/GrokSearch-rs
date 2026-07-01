use grok_search_tools::ToolSpec;
use grok_search_types::GrokSearchError;
use rmcp::model::Tool;
use rmcp::ErrorData as McpError;

pub(crate) fn tool_from_spec(spec: ToolSpec) -> Tool {
    Tool::new(spec.name, spec.description, spec.input_schema)
}

pub(crate) fn mcp_error(error: GrokSearchError) -> McpError {
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
