use clap::Parser;
use grok_search_apps::{
    InitOptions, McpServiceCommand as AppsMcpServiceCommand, McpServiceInstallOptions,
    McpServiceOptions,
};
use grok_search_config::Config;
use grok_search_tools::{
    AcademicCitationsParams, AcademicDownloadPdfParams, AcademicGetParams, AcademicParsePdfParams,
    AcademicPdfArtifactsParams, AcademicPdfDownloadParams, AcademicPdfReadParams,
    AcademicPdfStructureParams, AcademicProgressiveGetParams, AcademicReadParams,
    AcademicSearchParams, GetSourcesParams, RepoMetadataParams, WebFetchParams, WebMapParams,
    WebSearchParams, WechatSearchParams, ZhihuSearchParams,
};

use crate::args::{
    AcademicCommand, AcademicSubcommand, Cli, Command, InitCommand, InitTargetArg,
    McpServiceCommand, McpServiceSubcommand,
};
use crate::auth::{run_login, run_logout, run_status};
use crate::mcp::{run_mcp, run_mcp_http};
use crate::output::{invoke_and_print, print_json};
use crate::service::build_service;

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
        Some(Command::McpHttp(command)) => run_mcp_http(command).await,
        Some(Command::McpService(command)) => run_mcp_service(command),
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

fn run_mcp_service(command: McpServiceCommand) -> anyhow::Result<()> {
    let options = match command.command {
        McpServiceSubcommand::Install(command) => {
            let cfg = Config::try_load()?;
            let bind = match command.bind {
                Some(bind) => bind,
                None => cfg.mcp_http_bind.parse()?,
            };
            let path = command.path.unwrap_or_else(|| cfg.mcp_http_path.clone());
            let allow_origin = command
                .allow_origin
                .or_else(|| cfg.mcp_http_allow_origin.clone());
            McpServiceOptions {
                name: Some(command.name),
                command: AppsMcpServiceCommand::Install(McpServiceInstallOptions {
                    bind,
                    path,
                    allow_origin,
                    install_dir: command.install_dir,
                    auth_token_configured: cfg.mcp_http_auth_token.is_some(),
                    no_start: command.no_start,
                }),
            }
        }
        McpServiceSubcommand::Uninstall(command) => McpServiceOptions {
            name: Some(command.name),
            command: AppsMcpServiceCommand::Uninstall,
        },
        McpServiceSubcommand::Start(command) => McpServiceOptions {
            name: Some(command.name),
            command: AppsMcpServiceCommand::Start,
        },
        McpServiceSubcommand::Stop(command) => McpServiceOptions {
            name: Some(command.name),
            command: AppsMcpServiceCommand::Stop,
        },
        McpServiceSubcommand::Status(command) => McpServiceOptions {
            name: Some(command.name),
            command: AppsMcpServiceCommand::Status,
        },
    };

    let report = grok_search_apps::run_mcp_service(options)?;
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
                    cache_policy: command.cache_policy.map(Into::into),
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
                    cache_policy: command.cache_policy.map(Into::into),
                    vision_profile: command
                        .vision_profile
                        .map(|value| value.as_str().to_string()),
                    vision_max_pages: command.vision_max_pages,
                    vision_render_dpi: command.vision_render_dpi,
                    vision_concurrency: command.vision_concurrency,
                    vision_cache_policy: command.vision_cache_policy.map(Into::into),
                    vision_dir: command.vision_dir,
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
                    cache_policy: command.cache_policy.map(Into::into),
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
