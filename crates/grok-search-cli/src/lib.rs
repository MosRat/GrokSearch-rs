use std::io::{IsTerminal, Write};

use clap::{Args, Parser, Subcommand, ValueEnum};
use grok_search_apps::{InitOptions, InitTarget};
use grok_search_config::{self as config, AuthMode, Config};
use grok_search_tools::{
    AcademicCitationsParams, AcademicGetParams, AcademicReadParams, AcademicSearchParams,
    GetSourcesParams, WebFetchParams, WebMapParams, WebSearchParams,
};
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(
    name = "grok-search-rs",
    version,
    about = "GrokSearch-rs MCP server and CLI"
)]
struct Cli {
    /// Backward-compatible alias for `grok-search-rs init`.
    #[arg(long = "init", hide = true)]
    init_alias: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the MCP stdio server.
    Mcp,
    /// Initialize shared config and thin agent MCP entries.
    Init(InitCommand),
    /// Run xAI OAuth login.
    Login,
    /// Show OAuth status.
    Status,
    /// Remove local OAuth token file.
    Logout,
    /// Run connectivity diagnostics.
    Doctor(OutputArgs),
    /// Run web_search once and print JSON.
    #[command(name = "web-search", alias = "web_search")]
    WebSearch(WebSearchCommand),
    /// Return cached sources for a previous web_search session.
    #[command(name = "get-sources", alias = "get_sources")]
    GetSources(GetSourcesCommand),
    /// Fetch one URL as cleaned content.
    #[command(name = "web-fetch", alias = "web_fetch")]
    WebFetch(WebFetchCommand),
    /// Discover URLs on a site/domain.
    #[command(name = "web-map", alias = "web_map")]
    WebMap(WebMapCommand),
    /// Academic literature tools.
    Academic(AcademicCommand),
}

#[derive(Debug, Args)]
struct InitCommand {
    #[arg(long, value_enum, default_value_t = InitTargetArg::All)]
    target: InitTargetArg,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InitTargetArg {
    All,
    Codex,
    ClaudeCode,
    Snippets,
}

impl From<InitTargetArg> for InitTarget {
    fn from(value: InitTargetArg) -> Self {
        match value {
            InitTargetArg::All => InitTarget::All,
            InitTargetArg::Codex => InitTarget::Codex,
            InitTargetArg::ClaudeCode => InitTarget::ClaudeCode,
            InitTargetArg::Snippets => InitTarget::Snippets,
        }
    }
}

#[derive(Debug, Args)]
struct OutputArgs {
    #[arg(long)]
    compact: bool,
}

#[derive(Debug, Args)]
struct WebSearchCommand {
    query: String,
    #[arg(long)]
    platform: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    extra_sources: Option<usize>,
    #[arg(long)]
    recency_days: Option<u32>,
    #[arg(long = "include-domain")]
    include_domains: Vec<String>,
    #[arg(long = "exclude-domain")]
    exclude_domains: Vec<String>,
    #[arg(long)]
    include_content: Option<bool>,
    #[arg(long)]
    response_format: Option<String>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct GetSourcesCommand {
    session_id: String,
    #[arg(long)]
    offset: Option<usize>,
    #[arg(long)]
    limit: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct WebFetchCommand {
    url: String,
    #[arg(long)]
    max_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct WebMapCommand {
    url: String,
    #[arg(long)]
    max_results: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicCommand {
    #[command(subcommand)]
    command: AcademicSubcommand,
}

#[derive(Debug, Subcommand)]
enum AcademicSubcommand {
    /// Search computer-science literature.
    Search(AcademicSearchCommand),
    /// Resolve one academic paper.
    Get(AcademicGetCommand),
    /// Return citation/reference summaries.
    Citations(AcademicCitationsCommand),
    /// Resolve and parse academic full text.
    Read(AcademicReadCommand),
}

#[derive(Debug, Args)]
struct AcademicSearchCommand {
    query: String,
    #[arg(long = "source")]
    sources: Vec<String>,
    #[arg(long)]
    search_mode: Option<String>,
    #[arg(long)]
    sort_by: Option<String>,
    #[arg(long)]
    max_results: Option<usize>,
    #[arg(long)]
    year_from: Option<u32>,
    #[arg(long)]
    year_to: Option<u32>,
    #[arg(long)]
    open_access_only: bool,
    #[arg(long)]
    include_abstract: Option<bool>,
    #[arg(long)]
    include_citations: bool,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicGetCommand {
    identifier: String,
    #[arg(long)]
    include_citations: bool,
    #[arg(long)]
    include_open_access: Option<bool>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicCitationsCommand {
    identifier: String,
    #[arg(long)]
    limit: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicReadCommand {
    #[arg(long)]
    identifier: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    output_format: Option<String>,
    #[command(flatten)]
    output: OutputArgs,
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command = if cli.init_alias {
        Some(Command::Init(InitCommand {
            target: InitTargetArg::All,
            dry_run: false,
        }))
    } else {
        cli.command
    };

    match command {
        None | Some(Command::Mcp) => run_mcp().await,
        Some(Command::Init(command)) => run_init(command),
        Some(Command::Login) => {
            let cfg = Config::load();
            run_login(&cfg).await
        }
        Some(Command::Status) => {
            let cfg = Config::load();
            run_status(&cfg)
        }
        Some(Command::Logout) => {
            let cfg = Config::load();
            run_logout(&cfg)
        }
        Some(Command::Doctor(output)) => {
            let service = build_service().await?;
            print_json(service.doctor().await, output.compact)
        }
        Some(Command::WebSearch(command)) => {
            invoke_and_print(
                "web_search",
                WebSearchParams {
                    query: command.query,
                    platform: command.platform,
                    model: command.model,
                    extra_sources: command.extra_sources,
                    recency_days: command.recency_days,
                    include_domains: command.include_domains,
                    exclude_domains: command.exclude_domains,
                    include_content: command.include_content,
                    response_format: command.response_format,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::GetSources(command)) => {
            invoke_and_print(
                "get_sources",
                GetSourcesParams {
                    session_id: command.session_id,
                    offset: command.offset,
                    limit: command.limit,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::WebFetch(command)) => {
            invoke_and_print(
                "web_fetch",
                WebFetchParams {
                    url: command.url,
                    max_chars: command.max_chars,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::WebMap(command)) => {
            invoke_and_print(
                "web_map",
                WebMapParams {
                    url: command.url,
                    max_results: command.max_results,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::Academic(command)) => run_academic(command).await,
    }
}

async fn run_mcp() -> anyhow::Result<()> {
    let cfg = Config::load();
    if cfg.grok_auth_mode == AuthMode::ApiKey
        && cfg.grok_api_key.is_none()
        && std::io::stdin().is_terminal()
    {
        print_setup_guide();
        return Ok(());
    }
    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await;
    let service = grok_search_runtime::new_with_http(cfg, http, proxy_diagnostics)?;
    grok_search_mcp::run_stdio(service).await?;
    Ok(())
}

fn run_init(command: InitCommand) -> anyhow::Result<()> {
    let report = grok_search_apps::run_init(InitOptions {
        target: command.target.into(),
        dry_run: command.dry_run,
    })?;
    for message in report.messages {
        println!("{message}");
    }
    Ok(())
}

async fn run_academic(command: AcademicCommand) -> anyhow::Result<()> {
    match command.command {
        AcademicSubcommand::Search(command) => {
            invoke_and_print(
                "academic_search",
                AcademicSearchParams {
                    query: command.query,
                    sources: command.sources,
                    search_mode: command.search_mode,
                    sort_by: command.sort_by,
                    max_results: command.max_results,
                    year_from: command.year_from,
                    year_to: command.year_to,
                    open_access_only: command.open_access_only.then_some(true),
                    include_abstract: command.include_abstract,
                    include_citations: command.include_citations.then_some(true),
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::Get(command) => {
            invoke_and_print(
                "academic_get",
                AcademicGetParams {
                    identifier: command.identifier,
                    include_citations: command.include_citations.then_some(true),
                    include_open_access: command.include_open_access,
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::Citations(command) => {
            invoke_and_print(
                "academic_citations",
                AcademicCitationsParams {
                    identifier: command.identifier,
                    limit: command.limit,
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::Read(command) => {
            invoke_and_print(
                "academic_read",
                AcademicReadParams {
                    identifier: command.identifier,
                    url: command.url,
                    max_chars: command.max_chars,
                    output_format: command.output_format,
                },
                command.output.compact,
            )
            .await
        }
    }
}

async fn invoke_and_print<T: serde::Serialize>(
    name: &str,
    params: T,
    compact: bool,
) -> anyhow::Result<()> {
    let service = build_service().await?;
    let args = serde_json::to_value(params)?;
    let value = grok_search_tools::invoke_tool(&service, name, args).await?;
    print_json(value, compact)
}

async fn build_service() -> anyhow::Result<grok_search_service::SearchService> {
    let cfg = Config::load();
    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await;
    Ok(grok_search_runtime::new_with_http(
        cfg,
        http,
        proxy_diagnostics,
    )?)
}

fn print_json(value: Value, compact: bool) -> anyhow::Result<()> {
    let text = if compact {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    println!("{text}");
    Ok(())
}

async fn run_login(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let store = grok_search_auth::oauth::login::login(&path, true).await?;
    println!("Login successful.");
    println!("Auth file: {}", path.display());
    if let Some(exp) = grok_search_auth::oauth::token_store::jwt_exp(&store.access_token) {
        println!("Access token expires at unix time: {exp}");
    }
    Ok(())
}

fn run_status(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let status = grok_search_auth::oauth::token_store::auth_status(&path);
    println!("grok-search-rs OAuth status");
    println!("  Auth file: {}", status.path.display());
    println!(
        "  Authenticated: {}",
        if status.authenticated { "yes" } else { "no" }
    );
    println!(
        "  Refresh token: {}",
        if status.refresh_token_present {
            "present"
        } else {
            "missing"
        }
    );
    println!(
        "  Access expires at: {}",
        status
            .access_expires_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    println!(
        "  Base URL: {}",
        status.base_url.unwrap_or_else(|| "unknown".to_string())
    );
    Ok(())
}

fn run_logout(cfg: &Config) -> anyhow::Result<()> {
    let path = resolve_auth_path(cfg)?;
    let removed = grok_search_auth::oauth::token_store::delete_token_store(&path)?;
    if removed {
        println!("Removed OAuth token file: {}", path.display());
    } else {
        println!("No OAuth token file found: {}", path.display());
    }
    Ok(())
}

fn resolve_auth_path(cfg: &Config) -> anyhow::Result<std::path::PathBuf> {
    cfg.grok_auth_file
        .clone()
        .or_else(config::auth_path)
        .ok_or_else(|| anyhow::anyhow!("cannot resolve OAuth auth path; set GROK_SEARCH_AUTH_FILE"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_init_alias() {
        let cli = Cli::try_parse_from(["grok-search-rs", "--init"]).unwrap();
        assert!(cli.init_alias);
    }

    #[test]
    fn parses_mcp_and_oauth_commands() {
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "mcp"])
                .unwrap()
                .command,
            Some(Command::Mcp)
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "login"])
                .unwrap()
                .command,
            Some(Command::Login)
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "status"])
                .unwrap()
                .command,
            Some(Command::Status)
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "logout"])
                .unwrap()
                .command,
            Some(Command::Logout)
        ));
    }

    #[test]
    fn parses_web_tool_commands() {
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "web-search",
                "rust mcp",
                "--include-domain",
                "example.com",
                "--exclude-domain",
                "old.example",
                "--include-content",
                "false",
                "--compact",
            ])
            .unwrap()
            .command,
            Some(Command::WebSearch(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "get-sources", "abc", "--limit", "2"])
                .unwrap()
                .command,
            Some(Command::GetSources(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "web-fetch", "https://example.com"])
                .unwrap()
                .command,
            Some(Command::WebFetch(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "web-map", "https://example.com"])
                .unwrap()
                .command,
            Some(Command::WebMap(_))
        ));
    }

    #[test]
    fn parses_academic_tool_commands() {
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "search",
                "retrieval augmented generation",
                "--source",
                "dblp",
                "--source",
                "arxiv",
                "--include-citations",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(_))
        ));
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "academic", "get", "10.1145/123"])
                .unwrap()
                .command,
            Some(Command::Academic(_))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "citations",
                "arXiv:1706.03762"
            ])
            .unwrap()
            .command,
            Some(Command::Academic(_))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "read",
                "--url",
                "https://arxiv.org/pdf/1706.03762",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(_))
        ));
    }
}
