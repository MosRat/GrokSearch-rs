use grok_search_service::SearchService;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

use crate::handler::McpServer;

pub async fn run_stdio(service: SearchService) -> anyhow::Result<()> {
    let server = McpServer::new(service).serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
