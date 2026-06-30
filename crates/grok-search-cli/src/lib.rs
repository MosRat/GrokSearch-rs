use std::io::{IsTerminal, Write};

use clap::{Args, Parser, Subcommand, ValueEnum};
use grok_search_apps::{InitOptions, InitTarget};
use grok_search_config::{self as config, AuthMode, Config};
use grok_search_tools::{
    AcademicCitationsParams, AcademicDownloadPdfParams, AcademicGetParams,
    AcademicLlmProgressiveOptionsParams, AcademicParseOptionsParams, AcademicParsePdfParams,
    AcademicPdfArtifactsParams, AcademicPdfCachePolicyParam, AcademicPdfDownloadParams,
    AcademicPdfReadParams, AcademicPdfStructureParams, AcademicPdfStructureProfileParam,
    AcademicProgressiveGetParams, AcademicReadParams, AcademicSearchParams, GetSourcesParams,
    RepoMetadataParams, RepoProviderParam, WebFetchParams, WebMapParams, WebSearchParams,
    WechatSearchParams, ZhihuSearchParams,
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
    #[arg(long)]
    verbose: bool,
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
struct WechatSearchCommand {
    query: String,
    #[arg(long)]
    account: Option<String>,
    #[arg(long)]
    max_results: Option<usize>,
    #[arg(long)]
    pages: Option<usize>,
    #[arg(long)]
    include_content: Option<bool>,
    #[arg(long)]
    max_content_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct ZhihuSearchCommand {
    query: String,
    #[arg(long)]
    count: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct RepoMetadataCommand {
    #[arg(long)]
    url: Option<String>,
    #[arg(long, value_enum)]
    provider: Option<RepoProviderArg>,
    #[arg(long)]
    repo_id: Option<String>,
    #[arg(long)]
    owner: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    repo_type: Option<String>,
    #[arg(long)]
    include_readme: bool,
    #[arg(long)]
    include_card: bool,
    #[arg(long)]
    max_text_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RepoProviderArg {
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
    /// Read academic PDF text.
    #[command(name = "pdf-read", alias = "pdf_read")]
    PdfRead(AcademicPdfReadCommand),
    /// Generate or read progressive PDF structure.
    #[command(name = "pdf-structure", alias = "pdf_structure")]
    PdfStructure(AcademicPdfStructureCommand),
    /// Extract academic PDF images and tables.
    #[command(name = "pdf-artifacts", alias = "pdf_artifacts")]
    PdfArtifacts(AcademicPdfArtifactsCommand),
    /// Download an academic PDF without parsing.
    #[command(name = "pdf-download", alias = "pdf_download")]
    PdfDownload(AcademicPdfDownloadCommand),
    /// Resolve and parse academic full text.
    #[command(hide = true)]
    Read(AcademicReadCommand),
    /// Resolve academic full text and export parse artifacts.
    #[command(name = "parse-pdf", alias = "parse_pdf", hide = true)]
    ParsePdf(AcademicParsePdfCommand),
    /// Resolve and download an academic PDF without parsing.
    #[command(name = "download-pdf", alias = "download_pdf", hide = true)]
    DownloadPdf(AcademicDownloadPdfCommand),
    /// Read a cached progressive PDF reading structure.
    #[command(name = "progressive-get", alias = "progressive_get", hide = true)]
    ProgressiveGet(AcademicProgressiveGetCommand),
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
    #[arg(long)]
    extract_material_links: bool,
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

#[derive(Debug, Args, Default)]
struct AcademicPdfLocatorArgs {
    #[arg(long)]
    identifier: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    pdf_url: Option<String>,
}

#[derive(Debug, Args)]
struct AcademicPdfReadCommand {
    #[command(flatten)]
    locator: AcademicPdfLocatorArgs,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    text_mode: Option<String>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    include_raw_content: bool,
    #[arg(long)]
    include_processing: bool,
    #[arg(long)]
    extract_material_links: bool,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicPdfStructureCommand {
    #[command(flatten)]
    locator: AcademicPdfLocatorArgs,
    #[arg(long, value_parser = ["summary", "full", "section"])]
    view: Option<String>,
    #[arg(long)]
    section_id: Option<String>,
    #[arg(long, value_enum)]
    profile: Option<AcademicPdfStructureProfileArg>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_enum)]
    cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[arg(long)]
    include_section_text: bool,
    #[arg(long)]
    save_json_path: Option<String>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AcademicPdfStructureProfileArg {
    Fast,
    Balanced,
    Strict,
}

impl From<AcademicPdfStructureProfileArg> for AcademicPdfStructureProfileParam {
    fn from(value: AcademicPdfStructureProfileArg) -> Self {
        match value {
            AcademicPdfStructureProfileArg::Fast => AcademicPdfStructureProfileParam::Fast,
            AcademicPdfStructureProfileArg::Balanced => AcademicPdfStructureProfileParam::Balanced,
            AcademicPdfStructureProfileArg::Strict => AcademicPdfStructureProfileParam::Strict,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AcademicPdfCachePolicyArg {
    Auto,
    Refresh,
    Bypass,
}

impl From<AcademicPdfCachePolicyArg> for AcademicPdfCachePolicyParam {
    fn from(value: AcademicPdfCachePolicyArg) -> Self {
        match value {
            AcademicPdfCachePolicyArg::Auto => AcademicPdfCachePolicyParam::Auto,
            AcademicPdfCachePolicyArg::Refresh => AcademicPdfCachePolicyParam::Refresh,
            AcademicPdfCachePolicyArg::Bypass => AcademicPdfCachePolicyParam::Bypass,
        }
    }
}

#[derive(Debug, Args)]
struct AcademicPdfArtifactsCommand {
    #[command(flatten)]
    locator: AcademicPdfLocatorArgs,
    #[arg(long)]
    images_dir: Option<String>,
    #[arg(long)]
    tables_dir: Option<String>,
    #[arg(long)]
    extract_images: bool,
    #[arg(long)]
    extract_tables: bool,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    text_mode: Option<String>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicPdfDownloadCommand {
    #[command(flatten)]
    locator: AcademicPdfLocatorArgs,
    #[arg(long)]
    output_path: String,
    #[arg(long)]
    overwrite: bool,
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
    parse: AcademicParseOptionsCommand,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicParsePdfCommand {
    #[arg(long)]
    identifier: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    output_format: Option<String>,
    #[command(flatten)]
    parse: AcademicParseOptionsCommand,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicDownloadPdfCommand {
    #[arg(long)]
    identifier: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    output_path: String,
    #[arg(long)]
    overwrite: bool,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args)]
struct AcademicProgressiveGetCommand {
    cache_key: String,
    #[arg(long, value_parser = ["summary", "full", "section"])]
    view: Option<String>,
    #[arg(long)]
    section_id: Option<String>,
    #[arg(long)]
    include_section_text: bool,
    #[arg(long)]
    max_chars: Option<usize>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Debug, Args, Default)]
struct AcademicParseOptionsCommand {
    #[arg(long)]
    save_markdown_path: Option<String>,
    #[arg(long)]
    save_raw_content_path: Option<String>,
    #[arg(long)]
    images_dir: Option<String>,
    #[arg(long)]
    tables_dir: Option<String>,
    #[arg(long)]
    extract_images: bool,
    #[arg(long)]
    extract_tables: bool,
    #[arg(long)]
    extract_material_links: bool,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    text_processing_mode: Option<String>,
    #[arg(long)]
    include_raw_content: bool,
    #[arg(long)]
    llm_progressive: bool,
    #[arg(long)]
    llm_progressive_model: Option<String>,
    #[arg(long)]
    llm_progressive_max_chunk_chars: Option<usize>,
    #[arg(long)]
    llm_progressive_overlap_chars: Option<usize>,
    #[arg(long)]
    llm_progressive_concurrency: Option<usize>,
    #[arg(long)]
    llm_progressive_max_output_tokens: Option<u32>,
    #[arg(long, value_parser = ["md_light_plain_refs"])]
    llm_progressive_input_profile: Option<String>,
    #[arg(long, value_parser = ["compact_v2"])]
    llm_progressive_prompt_profile: Option<String>,
    #[arg(long)]
    llm_progressive_cache_enabled: Option<bool>,
    #[arg(long)]
    llm_progressive_cache_refresh: bool,
    #[arg(long)]
    llm_progressive_save_json_path: Option<String>,
    #[arg(long)]
    llm_progressive_include_section_text: bool,
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
            let cfg = Config::try_load()?;
            run_login(&cfg).await
        }
        Some(Command::Status) => {
            let cfg = Config::try_load()?;
            run_status(&cfg)
        }
        Some(Command::Logout) => {
            let cfg = Config::try_load()?;
            run_logout(&cfg)
        }
        Some(Command::Doctor(output)) => {
            let service = build_service().await?;
            print_json(
                service.doctor_with_options(output.verbose).await,
                output.compact,
            )
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
        Some(Command::WechatSearch(command)) => {
            invoke_and_print(
                "wechat_search",
                WechatSearchParams {
                    query: command.query,
                    account: command.account,
                    max_results: command.max_results,
                    pages: command.pages,
                    include_content: command.include_content,
                    max_content_chars: command.max_content_chars,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::ZhihuSearch(command)) => {
            invoke_and_print(
                "zhihu_search",
                ZhihuSearchParams {
                    query: command.query,
                    count: command.count,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::RepoMetadata(command)) => {
            invoke_and_print(
                "repo_metadata",
                RepoMetadataParams {
                    url: command.url,
                    provider: command.provider.map(Into::into),
                    repo_id: command.repo_id,
                    owner: command.owner,
                    name: command.name,
                    repo_type: command.repo_type,
                    include_readme: command.include_readme.then_some(true),
                    include_card: command.include_card.then_some(true),
                    max_text_chars: command.max_text_chars,
                },
                command.output.compact,
            )
            .await
        }
        Some(Command::Academic(command)) => run_academic(command).await,
    }
}

async fn run_mcp() -> anyhow::Result<()> {
    let cfg = Config::try_load()?;
    if cfg.grok_auth_mode == AuthMode::ApiKey
        && cfg.grok_api_key.is_none()
        && std::io::stdin().is_terminal()
    {
        print_setup_guide();
        return Ok(());
    }
    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await?;
    let service = grok_search_runtime::new_with_http(cfg, http, proxy_diagnostics)?;
    service.warm_academic_institutional_access();
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
                    extract_material_links: None,
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
                    extract_material_links: command.extract_material_links.then_some(true),
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
        AcademicSubcommand::PdfRead(command) => {
            invoke_and_print(
                "academic_pdf_read",
                AcademicPdfReadParams {
                    identifier: command.locator.identifier,
                    url: command.locator.url,
                    pdf_url: command.locator.pdf_url,
                    text_mode: command.text_mode,
                    max_chars: command.max_chars,
                    include_raw_content: command.include_raw_content.then_some(true),
                    include_processing: command.include_processing.then_some(true),
                    extract_material_links: command.extract_material_links.then_some(true),
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::PdfStructure(command) => {
            invoke_and_print(
                "academic_pdf_structure",
                AcademicPdfStructureParams {
                    identifier: command.locator.identifier,
                    url: command.locator.url,
                    pdf_url: command.locator.pdf_url,
                    view: command.view,
                    section_id: command.section_id,
                    profile: command.profile.map(Into::into),
                    model: command.model,
                    cache_policy: command.cache_policy.map(Into::into),
                    include_section_text: command.include_section_text.then_some(true),
                    save_json_path: command.save_json_path,
                    max_chars: command.max_chars,
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::PdfArtifacts(command) => {
            invoke_and_print(
                "academic_pdf_artifacts",
                AcademicPdfArtifactsParams {
                    identifier: command.locator.identifier,
                    url: command.locator.url,
                    pdf_url: command.locator.pdf_url,
                    images_dir: command.images_dir,
                    tables_dir: command.tables_dir,
                    extract_images: command.extract_images.then_some(true),
                    extract_tables: command.extract_tables.then_some(true),
                    text_mode: command.text_mode,
                    max_chars: command.max_chars,
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::PdfDownload(command) => {
            invoke_and_print(
                "academic_pdf_download",
                AcademicPdfDownloadParams {
                    identifier: command.locator.identifier,
                    url: command.locator.url,
                    pdf_url: command.locator.pdf_url,
                    output_path: command.output_path,
                    overwrite: command.overwrite.then_some(true),
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
                    parse_options: command.parse.into_options(),
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::ParsePdf(command) => {
            invoke_and_print(
                "academic_parse_pdf",
                AcademicParsePdfParams {
                    identifier: command.identifier,
                    url: command.url,
                    max_chars: command.max_chars,
                    output_format: command.output_format,
                    parse_options: command.parse.into_options(),
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::DownloadPdf(command) => {
            invoke_and_print(
                "academic_download_pdf",
                AcademicDownloadPdfParams {
                    identifier: command.identifier,
                    url: command.url,
                    output_path: command.output_path,
                    overwrite: command.overwrite.then_some(true),
                },
                command.output.compact,
            )
            .await
        }
        AcademicSubcommand::ProgressiveGet(command) => {
            invoke_and_print(
                "academic_progressive_get",
                AcademicProgressiveGetParams {
                    cache_key: command.cache_key,
                    view: command.view,
                    section_id: command.section_id,
                    include_section_text: command.include_section_text.then_some(true),
                    max_chars: command.max_chars,
                },
                command.output.compact,
            )
            .await
        }
    }
}

impl AcademicParseOptionsCommand {
    fn into_options(self) -> Option<AcademicParseOptionsParams> {
        if self.save_markdown_path.is_none()
            && self.save_raw_content_path.is_none()
            && self.images_dir.is_none()
            && self.tables_dir.is_none()
            && !self.extract_images
            && !self.extract_tables
            && !self.extract_material_links
            && self.text_processing_mode.is_none()
            && !self.include_raw_content
            && !self.llm_progressive
            && self.llm_progressive_model.is_none()
            && self.llm_progressive_max_chunk_chars.is_none()
            && self.llm_progressive_overlap_chars.is_none()
            && self.llm_progressive_concurrency.is_none()
            && self.llm_progressive_max_output_tokens.is_none()
            && self.llm_progressive_input_profile.is_none()
            && self.llm_progressive_prompt_profile.is_none()
            && self.llm_progressive_cache_enabled.is_none()
            && !self.llm_progressive_cache_refresh
            && self.llm_progressive_save_json_path.is_none()
            && !self.llm_progressive_include_section_text
        {
            return None;
        }
        let llm_progressive = (self.llm_progressive
            || self.llm_progressive_model.is_some()
            || self.llm_progressive_max_chunk_chars.is_some()
            || self.llm_progressive_overlap_chars.is_some()
            || self.llm_progressive_concurrency.is_some()
            || self.llm_progressive_max_output_tokens.is_some()
            || self.llm_progressive_input_profile.is_some()
            || self.llm_progressive_prompt_profile.is_some()
            || self.llm_progressive_cache_enabled.is_some()
            || self.llm_progressive_cache_refresh
            || self.llm_progressive_save_json_path.is_some()
            || self.llm_progressive_include_section_text)
            .then_some(AcademicLlmProgressiveOptionsParams {
                enabled: self.llm_progressive.then_some(true),
                model: self.llm_progressive_model,
                max_chunk_chars: self.llm_progressive_max_chunk_chars,
                overlap_chars: self.llm_progressive_overlap_chars,
                concurrency: self.llm_progressive_concurrency,
                max_output_tokens: self.llm_progressive_max_output_tokens,
                input_profile: self.llm_progressive_input_profile,
                prompt_profile: self.llm_progressive_prompt_profile,
                cache_enabled: self.llm_progressive_cache_enabled,
                cache_refresh: self.llm_progressive_cache_refresh.then_some(true),
                save_json_path: self.llm_progressive_save_json_path,
                include_section_text: self.llm_progressive_include_section_text.then_some(true),
            });
        Some(AcademicParseOptionsParams {
            save_markdown_path: self.save_markdown_path,
            save_raw_content_path: self.save_raw_content_path,
            images_dir: self.images_dir,
            tables_dir: self.tables_dir,
            extract_images: self.extract_images.then_some(true),
            extract_tables: self.extract_tables.then_some(true),
            extract_material_links: self.extract_material_links.then_some(true),
            text_processing_mode: self.text_processing_mode,
            include_raw_content: self.include_raw_content.then_some(true),
            llm_progressive,
        })
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
    let cfg = Config::try_load()?;
    let (http, proxy_diagnostics) = grok_search_net::proxy::bootstrap(&cfg).await?;
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
        match Cli::try_parse_from([
            "grok-search-rs",
            "wechat-search",
            "OpenAI",
            "--account",
            "机器之心",
            "--max-results",
            "5",
            "--pages",
            "2",
        ])
        .unwrap()
        .command
        {
            Some(Command::WechatSearch(command)) => {
                assert_eq!(command.query, "OpenAI");
                assert_eq!(command.account.as_deref(), Some("机器之心"));
                assert_eq!(command.max_results, Some(5));
                assert_eq!(command.pages, Some(2));
            }
            other => panic!("expected wechat search command, got {other:?}"),
        }
        match Cli::try_parse_from(["grok-search-rs", "zhihu-search", "OpenAI", "--count", "5"])
            .unwrap()
            .command
        {
            Some(Command::ZhihuSearch(command)) => {
                assert_eq!(command.query, "OpenAI");
                assert_eq!(command.count, Some(5));
            }
            other => panic!("expected zhihu search command, got {other:?}"),
        }
        assert!(matches!(
            Cli::try_parse_from(["grok-search-rs", "zhihu_search", "OpenAI"])
                .unwrap()
                .command,
            Some(Command::ZhihuSearch(_))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "repo-metadata",
                "--provider",
                "huggingface",
                "--repo-id",
                "bert-base-uncased",
                "--include-card"
            ])
            .unwrap()
            .command,
            Some(Command::RepoMetadata(_))
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
                "pdf-read",
                "--identifier",
                "arXiv:1706.03762",
                "--text-mode",
                "clean",
                "--include-raw-content",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(AcademicCommand {
                command: AcademicSubcommand::PdfRead(_)
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "pdf-structure",
                "--pdf-url",
                "https://arxiv.org/pdf/1706.03762",
                "--view",
                "section",
                "--section-id",
                "sec_000_intro",
                "--profile",
                "balanced",
                "--cache-policy",
                "refresh",
                "--include-section-text",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(AcademicCommand {
                command: AcademicSubcommand::PdfStructure(_)
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "pdf-artifacts",
                "--url",
                "https://arxiv.org/pdf/1706.03762",
                "--extract-images",
                "--images-dir",
                "images",
                "--extract-tables",
                "--tables-dir",
                "tables",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(AcademicCommand {
                command: AcademicSubcommand::PdfArtifacts(_)
            }))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "pdf-download",
                "--url",
                "https://arxiv.org/pdf/1706.03762",
                "--output-path",
                "paper.pdf",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(AcademicCommand {
                command: AcademicSubcommand::PdfDownload(_)
            }))
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
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "download-pdf",
                "--url",
                "https://arxiv.org/pdf/1706.03762",
                "--output-path",
                "paper.pdf",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(_))
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "grok-search-rs",
                "academic",
                "progressive-get",
                "progressive:v1:abc",
                "--view",
                "section",
                "--section-id",
                "sec_000_intro",
                "--include-section-text",
            ])
            .unwrap()
            .command,
            Some(Command::Academic(_))
        ));
        match Cli::try_parse_from([
            "grok-search-rs",
            "academic",
            "parse-pdf",
            "--url",
            "https://arxiv.org/pdf/1706.03762",
            "--text-processing-mode",
            "clean",
            "--include-raw-content",
            "--save-raw-content-path",
            "raw.md",
            "--llm-progressive",
            "--llm-progressive-model",
            "MiniMax-M3",
            "--llm-progressive-max-chunk-chars",
            "5000",
            "--llm-progressive-overlap-chars",
            "400",
            "--llm-progressive-concurrency",
            "2",
            "--llm-progressive-max-output-tokens",
            "1200",
            "--llm-progressive-input-profile",
            "md_light_plain_refs",
            "--llm-progressive-prompt-profile",
            "compact_v2",
            "--llm-progressive-cache-enabled",
            "true",
            "--llm-progressive-cache-refresh",
            "--llm-progressive-save-json-path",
            "progressive.json",
        ])
        .unwrap()
        .command
        {
            Some(Command::Academic(AcademicCommand {
                command: AcademicSubcommand::ParsePdf(command),
            })) => {
                assert_eq!(command.parse.text_processing_mode.as_deref(), Some("clean"));
                assert!(command.parse.include_raw_content);
                assert_eq!(
                    command.parse.save_raw_content_path.as_deref(),
                    Some("raw.md")
                );
                assert!(command.parse.llm_progressive);
                assert_eq!(
                    command.parse.llm_progressive_model.as_deref(),
                    Some("MiniMax-M3")
                );
                assert_eq!(command.parse.llm_progressive_max_chunk_chars, Some(5000));
                assert_eq!(command.parse.llm_progressive_overlap_chars, Some(400));
                assert_eq!(command.parse.llm_progressive_concurrency, Some(2));
                assert_eq!(command.parse.llm_progressive_max_output_tokens, Some(1200));
                assert_eq!(
                    command.parse.llm_progressive_input_profile.as_deref(),
                    Some("md_light_plain_refs")
                );
                assert_eq!(
                    command.parse.llm_progressive_prompt_profile.as_deref(),
                    Some("compact_v2")
                );
                assert_eq!(command.parse.llm_progressive_cache_enabled, Some(true));
                assert!(command.parse.llm_progressive_cache_refresh);
                assert_eq!(
                    command.parse.llm_progressive_save_json_path.as_deref(),
                    Some("progressive.json")
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
