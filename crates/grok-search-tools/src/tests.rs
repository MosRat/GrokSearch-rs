use crate::registry::TOOLS_SPEC_JSON;
use crate::*;
use grok_search_service::SearchService;
use grok_search_types::model::tool::WebSearchInput;
use grok_search_types::{AcademicParseOptions, AcademicSearchInput, GrokSearchError};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const EXPECTED_TOOLS: &[&str] = &[
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
    "academic_pdf_read",
    "academic_pdf_structure",
    "academic_pdf_artifacts",
    "academic_pdf_download",
];

#[test]
fn tools_list_contains_existing_tools() {
    let names: Vec<_> = tools().into_iter().map(|tool| tool.name).collect();
    assert_eq!(names, EXPECTED_TOOLS);
}

#[test]
fn embedded_tools_spec_has_valid_shape() {
    let spec: Value = serde_json::from_str(TOOLS_SPEC_JSON).expect("valid tools spec json");
    let tools = spec["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), EXPECTED_TOOLS.len());

    for (tool, expected_name) in tools.iter().zip(EXPECTED_TOOLS) {
        assert_eq!(tool["name"], json!(expected_name));
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|description| !description.trim().is_empty()),
            "{expected_name} description must be non-empty"
        );
        assert!(
            tool["inputSchema"].as_object().is_some(),
            "{expected_name} inputSchema must be an object"
        );
    }
}

#[test]
fn repo_local_skills_have_minimal_valid_layout() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root");
    let skills_dir = repo_root.join("skills");
    let expected_skills = [
        "grok-search-web-research",
        "grok-search-academic-literature",
        "grok-search-social-search",
        "grok-search-repo-intelligence",
        "grok-search-diagnostics",
    ];

    for name in expected_skills {
        let skill_dir = skills_dir.join(name);
        let skill_md_path = skill_dir.join("SKILL.md");
        let examples_path = skill_dir.join("references").join("examples.md");
        let agent_metadata_path = skill_dir.join("agents").join("openai.yaml");

        let skill_md = fs::read_to_string(&skill_md_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", skill_md_path.display()));
        assert!(
            skill_md.starts_with("---\n"),
            "{} must start with YAML frontmatter",
            skill_md_path.display()
        );
        let end = skill_md[4..]
            .find("\n---\n")
            .map(|idx| idx + 4)
            .expect("frontmatter terminator");
        let frontmatter = &skill_md[4..end];
        let keys: BTreeSet<_> = frontmatter
            .lines()
            .filter_map(|line| line.split_once(':').map(|(key, _)| key.trim()))
            .collect();
        assert_eq!(keys, BTreeSet::from(["description", "name"]));
        assert!(
            name.chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
            "{name} must be hyphen-case ASCII"
        );
        assert!(frontmatter.contains(&format!("name: {name}")));
        assert!(
            skill_md.contains("references/examples.md"),
            "{name} should point to examples reference"
        );
        assert!(
            examples_path.exists(),
            "{} missing",
            examples_path.display()
        );
        assert!(
            agent_metadata_path.exists(),
            "{} missing",
            agent_metadata_path.display()
        );
        let agent_metadata = fs::read_to_string(&agent_metadata_path)
            .unwrap_or_else(|err| panic!("read {}: {err}", agent_metadata_path.display()));
        for key in ["display_name", "short_description", "default_prompt"] {
            let prefix = format!("{key}: ");
            let value = agent_metadata
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .unwrap_or_else(|| panic!("{name} missing {key}"));
            assert!(!value.trim().is_empty(), "{name} {key} must be non-empty");
        }

        for banned in [
            skill_dir.join("README.md"),
            skill_dir.join("CHANGELOG.md"),
            skill_dir.join("INSTALLATION_GUIDE.md"),
            skill_dir.join("QUICK_REFERENCE.md"),
        ] {
            assert!(!banned.exists(), "{} should not exist", banned.display());
        }
    }
}

#[test]
fn typed_web_search_args_accept_existing_shape() {
    let params: WebSearchParams = serde_json::from_value(json!({
        "query": "rust mcp",
        "platform": "x",
        "model": "grok-4",
        "extra_sources": 2,
        "recency_days": 7,
        "include_domains": ["example.com"],
        "exclude_domains": ["old.example"],
        "include_content": true,
        "response_format": "detailed"
    }))
    .expect("valid params");

    let input: WebSearchInput = params.into();
    assert_eq!(input.query, "rust mcp");
    assert_eq!(input.extra_sources, Some(2));
    assert_eq!(input.recency_days, Some(7));
    assert_eq!(input.include_domains, vec!["example.com"]);
}

#[test]
fn typed_academic_args_accept_existing_shape() {
    let params: AcademicSearchParams = serde_json::from_value(json!({
        "query": "retrieval augmented generation",
        "sources": ["dblp", "arxiv"],
        "search_mode": "broad",
        "sort_by": "citations",
        "max_results": 5,
        "year_from": 2020,
        "year_to": 2026,
        "open_access_only": true,
        "include_abstract": true,
        "include_citations": false
    }))
    .expect("valid params");

    let input: AcademicSearchInput = params.into();
    assert_eq!(input.query, "retrieval augmented generation");
    assert_eq!(input.sources, vec!["dblp", "arxiv"]);
    assert_eq!(input.search_mode.as_deref(), Some("broad"));
    assert_eq!(input.sort_by.as_deref(), Some("citations"));
    assert_eq!(input.max_results, Some(5));
    assert_eq!(input.year_from, Some(2020));
}

#[test]
fn academic_search_schema_documents_semantic_scholar_alias() {
    let spec: Value = serde_json::from_str(TOOLS_SPEC_JSON).expect("valid tools spec json");
    let academic = spec["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["name"] == "academic_search")
        .expect("academic_search tool");
    let sources = academic["inputSchema"]["properties"]["sources"]["items"]["enum"]
        .as_array()
        .expect("sources enum");
    assert!(sources.contains(&json!("semantic")));
    assert!(sources.contains(&json!("semantic_scholar")));
}

#[test]
fn typed_academic_parse_options_accept_llm_progressive_shape() {
    let params: AcademicParseOptionsParams = serde_json::from_value(json!({
        "text_processing_mode": "clean",
        "llm_progressive": {
            "enabled": true,
            "model": "MiniMax-M3",
            "max_chunk_chars": 5000,
            "overlap_chars": 400,
            "concurrency": 2,
            "max_output_tokens": 1200,
            "input_profile": "md_light_plain_refs",
            "prompt_profile": "compact_v2",
            "cache_enabled": true,
            "cache_refresh": true,
            "save_json_path": "progressive.json",
            "include_section_text": false
        }
    }))
    .expect("valid parse options");

    let input: AcademicParseOptions = params.into();
    let llm = input.llm_progressive.expect("llm progressive options");
    assert_eq!(llm.enabled, Some(true));
    assert_eq!(llm.model.as_deref(), Some("MiniMax-M3"));
    assert_eq!(llm.max_chunk_chars, Some(5000));
    assert_eq!(llm.overlap_chars, Some(400));
    assert_eq!(llm.concurrency, Some(2));
    assert_eq!(llm.max_output_tokens, Some(1200));
    assert_eq!(llm.input_profile.as_deref(), Some("md_light_plain_refs"));
    assert_eq!(llm.prompt_profile.as_deref(), Some("compact_v2"));
    assert_eq!(llm.cache_enabled, Some(true));
    assert_eq!(llm.cache_refresh, Some(true));
    assert_eq!(llm.save_json_path.as_deref(), Some("progressive.json"));
}

#[test]
fn academic_pdf_read_schema_is_text_only() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "academic_pdf_read")
        .expect("academic_pdf_read tool");
    let properties = tool.input_schema["properties"].as_object().unwrap();
    for key in [
        "identifier",
        "url",
        "pdf_url",
        "text_mode",
        "max_chars",
        "include_raw_content",
        "include_processing",
        "extract_material_links",
        "cache_policy",
    ] {
        assert!(properties.contains_key(key), "missing {key}");
    }
    assert!(!properties.contains_key("parse_options"));
    assert!(!properties.contains_key("llm_progressive"));
}

#[test]
fn academic_pdf_structure_schema_hides_low_level_llm_knobs() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "academic_pdf_structure")
        .expect("academic_pdf_structure tool");
    let properties = tool.input_schema["properties"].as_object().unwrap();
    for key in [
        "identifier",
        "url",
        "pdf_url",
        "view",
        "section_id",
        "profile",
        "model",
        "cache_policy",
        "include_section_text",
        "save_json_path",
        "max_chars",
    ] {
        assert!(properties.contains_key(key), "missing {key}");
    }
    for hidden in [
        "max_chunk_chars",
        "overlap_chars",
        "max_output_tokens",
        "input_profile",
        "prompt_profile",
        "concurrency",
        "cache_enabled",
    ] {
        assert!(!properties.contains_key(hidden), "leaked {hidden}");
    }
}

#[test]
fn academic_pdf_artifacts_schema_is_artifact_only() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "academic_pdf_artifacts")
        .expect("academic_pdf_artifacts tool");
    let properties = tool.input_schema["properties"].as_object().unwrap();
    for key in [
        "identifier",
        "url",
        "pdf_url",
        "images_dir",
        "tables_dir",
        "extract_images",
        "extract_tables",
        "text_mode",
        "cache_policy",
    ] {
        assert!(properties.contains_key(key), "missing {key}");
    }
    assert!(!properties.contains_key("content"));
    assert!(!properties.contains_key("llm_progressive"));
}

#[test]
fn academic_pdf_download_schema_requires_output_path() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "academic_pdf_download")
        .expect("academic_pdf_download tool");
    let required = tool.input_schema["required"].as_array().unwrap();
    assert_eq!(required, &vec![json!("output_path")]);
    let properties = tool.input_schema["properties"].as_object().unwrap();
    assert!(properties.contains_key("identifier"));
    assert!(properties.contains_key("url"));
    assert!(properties.contains_key("pdf_url"));
    assert!(properties.contains_key("overwrite"));
    assert!(properties.contains_key("cache_policy"));
}

#[test]
fn legacy_pdf_stage_tools_are_not_agent_facing() {
    let names: BTreeSet<_> = tools().into_iter().map(|tool| tool.name).collect();
    for hidden in [
        "academic_read",
        "academic_parse_pdf",
        "academic_download_pdf",
        "academic_progressive_get",
    ] {
        assert!(!names.contains(hidden), "{hidden} should be hidden");
    }
}

#[test]
fn repo_metadata_schema_exposes_provider_and_text_flags() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "repo_metadata")
        .expect("repo_metadata tool");
    let properties = tool.input_schema["properties"].as_object().unwrap();
    assert!(properties.contains_key("url"));
    assert!(properties.contains_key("provider"));
    assert!(properties.contains_key("include_readme"));
    assert!(properties.contains_key("include_card"));
}

#[test]
fn wechat_search_schema_exposes_filters_and_content_flags() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "wechat_search")
        .expect("wechat_search tool");
    let required = tool.input_schema["required"].as_array().unwrap();
    assert_eq!(required, &vec![json!("query")]);
    let properties = tool.input_schema["properties"].as_object().unwrap();
    for key in [
        "query",
        "account",
        "max_results",
        "pages",
        "include_content",
        "max_content_chars",
    ] {
        assert!(properties.contains_key(key), "missing {key}");
    }
}

#[test]
fn zhihu_search_schema_exposes_query_and_count() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "zhihu_search")
        .expect("zhihu_search tool");
    let required = tool.input_schema["required"].as_array().unwrap();
    assert_eq!(required, &vec![json!("query")]);
    let properties = tool.input_schema["properties"].as_object().unwrap();
    assert!(properties.contains_key("query"));
    assert_eq!(properties["count"]["minimum"], json!(1));
    assert_eq!(properties["count"]["maximum"], json!(10));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_returns_existing_web_map_shape() {
    let service = SearchService::fake_with_sources();
    let value = invoke_tool(
        &service,
        "web_map",
        json!({
            "url": "https://93.184.216.34",
            "max_results": 2
        }),
    )
    .await
    .expect("web_map should succeed");

    assert_eq!(value["url"], "https://93.184.216.34");
    assert_eq!(value["sources_count"], 2);
    assert_eq!(value["sources"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_web_map_out_of_range() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "web_map",
        json!({
            "url": "https://93.184.216.34",
            "max_results": 51
        }),
    )
    .await
    .expect_err("max_results above 50 should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_wechat_search_invalid_params() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(&service, "wechat_search", json!({ "query": "   " }))
        .await
        .expect_err("empty query should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));

    let err = invoke_tool(
        &service,
        "wechat_search",
        json!({ "query": "OpenAI", "pages": 11 }),
    )
    .await
    .expect_err("pages above 10 should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));

    let err = invoke_tool(
        &service,
        "wechat_search",
        json!({ "query": "OpenAI", "max_results": 0 }),
    )
    .await
    .expect_err("max_results below 1 should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_zhihu_search_invalid_params() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(&service, "zhihu_search", json!({ "query": "   " }))
        .await
        .expect_err("empty query should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));

    let err = invoke_tool(
        &service,
        "zhihu_search",
        json!({ "query": "OpenAI", "count": 0 }),
    )
    .await
    .expect_err("count below 1 should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));

    let err = invoke_tool(
        &service,
        "zhihu_search",
        json!({ "query": "OpenAI", "count": 11 }),
    )
    .await
    .expect_err("count above 10 should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_academic_download_pdf_without_location() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "academic_pdf_download",
        json!({
            "output_path": "paper.pdf"
        }),
    )
    .await
    .expect_err("missing identifier/url should fail before service call");

    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    assert!(err
        .to_string()
        .contains("academic_pdf_download requires exactly one"));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_academic_download_pdf_without_output_path() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "academic_pdf_download",
        json!({
            "url": "https://arxiv.org/pdf/1706.03762",
            "output_path": ""
        }),
    )
    .await
    .expect_err("empty output_path should fail before service call");

    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    assert!(err
        .to_string()
        .contains("academic_pdf_download.output_path is required"));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_academic_pdf_structure_section_without_id() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "academic_pdf_structure",
        json!({
            "pdf_url": "https://arxiv.org/pdf/1706.03762",
            "view": "section"
        }),
    )
    .await
    .expect_err("section view without section_id should fail before service call");

    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    assert!(err
        .to_string()
        .contains("academic_pdf_structure.section_id is required"));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_rejects_academic_pdf_artifacts_missing_dirs() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "academic_pdf_artifacts",
        json!({
            "pdf_url": "https://arxiv.org/pdf/1706.03762",
            "extract_images": true
        }),
    )
    .await
    .expect_err("extract_images without images_dir should fail before service call");

    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    assert!(err
        .to_string()
        .contains("academic_pdf_artifacts.images_dir is required"));
}

#[tokio::test(flavor = "current_thread")]
async fn legacy_academic_progressive_get_dispatch_remains_available() {
    let service = SearchService::fake_with_sources();
    let err = invoke_tool(
        &service,
        "academic_progressive_get",
        json!({
            "cache_key": "",
        }),
    )
    .await
    .expect_err("empty cache key should still be validated");

    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    assert!(err
        .to_string()
        .contains("academic_progressive_get.cache_key is required"));
}

#[tokio::test(flavor = "current_thread")]
async fn invoke_tool_doctor_accepts_verbose_param() {
    let service = SearchService::fake_with_sources();
    let value = invoke_tool(&service, "doctor", json!({ "verbose": true }))
        .await
        .expect("doctor should succeed");

    assert_eq!(value["diagnostics"]["debug_log"]["enabled"], false);
    assert!(value["diagnostics"]["limits"]["max_response_bytes"].is_number());
}
