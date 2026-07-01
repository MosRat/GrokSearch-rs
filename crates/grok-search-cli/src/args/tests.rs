use std::net::SocketAddr;

use clap::Parser;

use super::{academic::*, *};

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
    match Cli::try_parse_from([
        "grok-search-rs",
        "mcp-http",
        "--bind",
        "127.0.0.1:0",
        "--path",
        "/mcp",
        "--allow-origin",
        "http://127.0.0.1:3000",
    ])
    .unwrap()
    .command
    {
        Some(Command::McpHttp(command)) => {
            assert_eq!(
                command.bind,
                Some("127.0.0.1:0".parse::<SocketAddr>().unwrap())
            );
            assert_eq!(command.path.as_deref(), Some("/mcp"));
            assert_eq!(
                command.allow_origin.as_deref(),
                Some("http://127.0.0.1:3000")
            );
        }
        other => panic!("expected mcp-http command, got {other:?}"),
    }
    assert!(matches!(
        Cli::try_parse_from(["grok-search-rs", "mcp_http"])
            .unwrap()
            .command,
        Some(Command::McpHttp(_))
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
            "--cache-policy",
            "refresh",
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
            "--cache-policy",
            "refresh",
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
            "--cache-policy",
            "refresh",
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
