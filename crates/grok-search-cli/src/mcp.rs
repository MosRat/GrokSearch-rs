use std::io::{IsTerminal, Write};

use grok_search_config::{self as config, AuthMode, Config};

use crate::args::McpHttpCommand;
use crate::service::build_service_from_config;

pub(crate) async fn run_mcp() -> anyhow::Result<()> {
    let cfg = Config::try_load()?;
    if cfg.grok_auth_mode == AuthMode::ApiKey
        && cfg.grok_api_key.is_none()
        && std::io::stdin().is_terminal()
    {
        print_setup_guide();
        return Ok(());
    }
    let service = build_service_from_config(cfg).await?;
    service.warm_academic_institutional_access();
    grok_search_mcp::run_stdio(service).await?;
    Ok(())
}

pub(crate) async fn run_mcp_http(command: McpHttpCommand) -> anyhow::Result<()> {
    let cfg = Config::try_load()?;
    let bind = match command.bind {
        Some(bind) => bind,
        None => cfg.mcp_http_bind.parse()?,
    };
    let path = command.path.unwrap_or_else(|| cfg.mcp_http_path.clone());
    let allow_origin = command
        .allow_origin
        .or_else(|| cfg.mcp_http_allow_origin.clone());
    let options = grok_search_mcp::McpHttpOptions::new(
        bind,
        path,
        cfg.mcp_http_auth_token.clone(),
        allow_origin,
    )?;
    let service = build_service_from_config(cfg).await?;
    service.warm_academic_institutional_access();
    grok_search_mcp::run_http(service, options).await
}

fn print_setup_guide() {
    let mut guide = String::from(
        r#"grok-search-rs is an MCP server. It speaks JSON-RPC over stdio and
should be launched by an MCP client (Claude Code, Codex CLI, Gemini CLI,
Cursor, VS Code, Windsurf, ...), not run directly.

Required keys
  GROK_SEARCH_API_KEY   xAI / Grok-compatible key   (https://x.ai/api)
  TAVILY_API_KEY        Tavily fetch + map          (https://tavily.com)
  FIRECRAWL_API_KEY     optional fetch fallback     (https://firecrawl.dev)

OAuth alternative
  grok-search-rs login
  Set grok_auth_mode = "oauth" in the global config.
  OAuth mode reuses Hermes' xAI client_id and may carry account / terms risk.

Recommended setup
  grok-search-rs init
  Fill the global config once, then keep each MCP client entry thin:
  {"type":"stdio","command":"grok-search-rs"}

"#,
    );

    if let Some(path) = config::config_path() {
        if !path.exists() {
            guide.push_str(&format!(
                r#"Tip: set keys once for every MCP client
  grok-search-rs init                    # scaffold config + thin agent entries
  $EDITOR {}    # uncomment and fill

"#,
                path.display()
            ));
        }
    }

    guide.push_str(
        r#"Docs:    https://github.com/MosRat/GrokSearch-rs#readme
Issues:  https://github.com/MosRat/GrokSearch-rs/issues
"#,
    );

    let stdout = std::io::stdout();
    let _ = stdout.lock().write_all(guide.as_bytes());
}
