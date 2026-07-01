use std::net::SocketAddr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use grok_search_apps::InitTarget;
use grok_search_tools::RepoProviderParam;

mod academic;
#[cfg(test)]
mod tests;

pub(crate) use academic::{AcademicCommand, AcademicSubcommand};

#[derive(Debug, Parser)]
#[command(
    name = "grok-search-rs",
    version,
    about = "GrokSearch-rs MCP server and CLI"
)]
pub(crate) struct Cli {
    /// Backward-compatible alias for `grok-search-rs init`.
    #[arg(long = "init", hide = true)]
    pub(crate) init_alias: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Start the MCP stdio server.
    Mcp,
    /// Start the MCP Streamable HTTP server.
    #[command(name = "mcp-http", alias = "mcp_http")]
    McpHttp(McpHttpCommand),
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
    /// Search WeChat public-account articles.
    #[command(name = "wechat-search", alias = "wechat_search")]
    WechatSearch(WechatSearchCommand),
    /// Search Zhihu site content through Zhihu OpenAPI.
    #[command(name = "zhihu-search", alias = "zhihu_search")]
    ZhihuSearch(ZhihuSearchCommand),
    /// Fetch GitHub or Hugging Face repository metadata.
    #[command(name = "repo-metadata", alias = "repo_metadata")]
    RepoMetadata(RepoMetadataCommand),
    /// Academic literature tools.
    Academic(AcademicCommand),
}

#[derive(Debug, Args)]
pub(crate) struct InitCommand {
    #[arg(long, value_enum, default_value_t = InitTargetArg::All)]
    pub(crate) target: InitTargetArg,
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum InitTargetArg {
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
pub(crate) struct OutputArgs {
    #[arg(long)]
    pub(crate) compact: bool,
    #[arg(long)]
    pub(crate) verbose: bool,
}

#[derive(Debug, Args)]
pub(crate) struct McpHttpCommand {
    #[arg(long)]
    pub(crate) bind: Option<SocketAddr>,
    #[arg(long)]
    pub(crate) path: Option<String>,
    #[arg(long = "allow-origin")]
    pub(crate) allow_origin: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct WebSearchCommand {
    pub(crate) query: String,
    #[arg(long)]
    pub(crate) platform: Option<String>,
    #[arg(long)]
    pub(crate) model: Option<String>,
    #[arg(long)]
    pub(crate) extra_sources: Option<usize>,
    #[arg(long)]
    pub(crate) recency_days: Option<u32>,
    #[arg(long = "include-domain")]
    pub(crate) include_domains: Vec<String>,
    #[arg(long = "exclude-domain")]
    pub(crate) exclude_domains: Vec<String>,
    #[arg(long)]
    pub(crate) include_content: Option<bool>,
    #[arg(long)]
    pub(crate) response_format: Option<String>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct GetSourcesCommand {
    pub(crate) session_id: String,
    #[arg(long)]
    pub(crate) offset: Option<usize>,
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct WebFetchCommand {
    pub(crate) url: String,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct WebMapCommand {
    pub(crate) url: String,
    #[arg(long)]
    pub(crate) max_results: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct WechatSearchCommand {
    pub(crate) query: String,
    #[arg(long)]
    pub(crate) account: Option<String>,
    #[arg(long)]
    pub(crate) max_results: Option<usize>,
    #[arg(long)]
    pub(crate) pages: Option<usize>,
    #[arg(long)]
    pub(crate) include_content: Option<bool>,
    #[arg(long)]
    pub(crate) max_content_chars: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct ZhihuSearchCommand {
    pub(crate) query: String,
    #[arg(long)]
    pub(crate) count: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct RepoMetadataCommand {
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) provider: Option<RepoProviderArg>,
    #[arg(long)]
    pub(crate) repo_id: Option<String>,
    #[arg(long)]
    pub(crate) owner: Option<String>,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) repo_type: Option<String>,
    #[arg(long)]
    pub(crate) include_readme: bool,
    #[arg(long)]
    pub(crate) include_card: bool,
    #[arg(long)]
    pub(crate) max_text_chars: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum RepoProviderArg {
    Github,
    Huggingface,
}

impl From<RepoProviderArg> for RepoProviderParam {
    fn from(value: RepoProviderArg) -> Self {
        match value {
            RepoProviderArg::Github => RepoProviderParam::Github,
            RepoProviderArg::Huggingface => RepoProviderParam::Huggingface,
        }
    }
}
