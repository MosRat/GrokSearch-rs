use grok_search_service::SearchService;
use grok_search_types::model::tool::{WebSearchInput, WechatSearchInput, ZhihuSearchInput};
use grok_search_types::{
    AcademicLlmProgressiveOptions, AcademicParseOptions, AcademicPdfArtifactsInput,
    AcademicPdfCachePolicy, AcademicPdfDownloadInput, AcademicPdfLocator, AcademicPdfReadInput,
    AcademicPdfStructureInput, AcademicPdfStructureProfile, AcademicProgressiveGetInput,
    AcademicSearchInput,
};
use grok_search_types::{GrokSearchError, RepoMetadataInput, RepoProvider, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Map<String, Value>,
}

pub fn tools() -> Vec<ToolSpec> {
    tools_list_json()["tools"]
        .as_array()
        .expect("tools_list is an array")
        .iter()
        .map(tool_from_value)
        .collect()
}

fn tool_from_value(value: &Value) -> ToolSpec {
    ToolSpec {
        name: value["name"].as_str().expect("tool name").to_string(),
        description: value["description"]
            .as_str()
            .expect("tool description")
            .to_string(),
        input_schema: value["inputSchema"]
            .as_object()
            .expect("input schema")
            .clone(),
    }
}

pub async fn invoke_tool(service: &SearchService, name: &str, args: Value) -> Result<Value> {
    match name {
        "doctor" => {
            let params: DoctorParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            Ok(service
                .doctor_with_options(params.verbose.unwrap_or(false))
                .await)
        }
        "web_search" => {
            let params: WebSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            validate_required_query("web_search", &params.query)?;
            let output = service.web_search(params.into()).await?;
            serialize_output(output, "serialize output")
        }
        "get_sources" => {
            let params: GetSourcesParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .get_sources(
                    &params.session_id,
                    params.offset.unwrap_or(0),
                    params.limit.filter(|value| *value > 0),
                )
                .await?;
            serialize_output(output, "serialize sources")
        }
        "web_fetch" => {
            let params: WebFetchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .web_fetch(&params.url, params.max_chars.filter(|value| *value > 0))
                .await?;
            serialize_output(output, "serialize fetch")
        }
        "web_map" => {
            let params: WebMapParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let max_results = params.max_results.unwrap_or(10);
            if !(1..=50).contains(&max_results) {
                return Err(GrokSearchError::InvalidParams(
                    "web_map.max_results must be between 1 and 50".to_string(),
                ));
            }
            let sources = service.web_map(&params.url, max_results).await?;
            let mapped_sources: Vec<Value> = sources
                .iter()
                .map(|source| json!({ "url": &source.url, "provider": &source.provider }))
                .collect();
            Ok(json!({
                "url": params.url,
                "sources_count": mapped_sources.len(),
                "sources": mapped_sources
            }))
        }
        "wechat_search" => {
            let params: WechatSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            validate_required_query("wechat_search", &params.query)?;
            validate_range(params.max_results, 1, 50, "wechat_search.max_results")?;
            validate_range(params.pages, 1, 10, "wechat_search.pages")?;
            if params.max_content_chars.is_some_and(|value| value == 0) {
                return Err(GrokSearchError::InvalidParams(
                    "wechat_search.max_content_chars must be greater than 0".into(),
                ));
            }
            let output = service.wechat_search(params.into()).await?;
            serialize_output(output, "serialize wechat search")
        }
        "zhihu_search" => {
            let params: ZhihuSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            validate_required_query("zhihu_search", &params.query)?;
            validate_range(params.count, 1, 10, "zhihu_search.count")?;
            let output = service.zhihu_search(params.into()).await?;
            serialize_output(output, "serialize zhihu search")
        }
        "repo_metadata" => {
            let params: RepoMetadataParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service.repo_metadata(params.into()).await?;
            serialize_output(output, "serialize repo metadata")
        }
        "academic_search" => {
            let params: AcademicSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            validate_required_query("academic_search", &params.query)?;
            let output = service.academic_search(params.into()).await?;
            serialize_output(output, "serialize academic search")
        }
        "academic_get" => {
            let params: AcademicGetParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_get.identifier is required".into(),
                ));
            }
            let output = service
                .academic_get(
                    &params.identifier,
                    params.include_citations.unwrap_or(false),
                    params.include_open_access.unwrap_or(true),
                    params.extract_material_links.unwrap_or(false),
                )
                .await?;
            serialize_output(output, "serialize academic get")
        }
        "academic_citations" => {
            let params: AcademicCitationsParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .academic_citations(&params.identifier, params.limit.filter(|value| *value > 0))
                .await?;
            serialize_output(output, "serialize academic citations")
        }
        "academic_pdf_read" => {
            let params: AcademicPdfReadParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let input: AcademicPdfReadInput = params.into();
            validate_pdf_locator("academic_pdf_read", &input.locator)?;
            validate_text_processing_mode("academic_pdf_read", input.text_mode.as_deref())?;
            let output = service.academic_pdf_read(input).await?;
            serialize_output(output, "serialize academic pdf read")
        }
        "academic_pdf_structure" => {
            let params: AcademicPdfStructureParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let input: AcademicPdfStructureInput = params.into();
            validate_pdf_locator("academic_pdf_structure", &input.locator)?;
            validate_structure_view("academic_pdf_structure", input.view.as_deref())?;
            if input.view.as_deref() == Some("section")
                && input.section_id.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_pdf_structure.section_id is required when view=section".into(),
                ));
            }
            let output = service.academic_pdf_structure(input).await?;
            serialize_output(output, "serialize academic pdf structure")
        }
        "academic_pdf_artifacts" => {
            let params: AcademicPdfArtifactsParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let input: AcademicPdfArtifactsInput = params.into();
            validate_pdf_locator("academic_pdf_artifacts", &input.locator)?;
            validate_text_processing_mode("academic_pdf_artifacts", input.text_mode.as_deref())?;
            if input.extract_images == Some(true)
                && input.images_dir.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_pdf_artifacts.images_dir is required when extract_images=true".into(),
                ));
            }
            if input.extract_tables == Some(true)
                && input.tables_dir.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_pdf_artifacts.tables_dir is required when extract_tables=true".into(),
                ));
            }
            let output = service.academic_pdf_artifacts(input).await?;
            serialize_output(output, "serialize academic pdf artifacts")
        }
        "academic_pdf_download" => {
            let params: AcademicPdfDownloadParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let input: AcademicPdfDownloadInput = params.into();
            validate_pdf_locator("academic_pdf_download", &input.locator)?;
            if input.output_path.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_pdf_download.output_path is required".into(),
                ));
            }
            let output = service.academic_pdf_download(input).await?;
            serialize_output(output, "serialize academic pdf download")
        }
        "academic_read" => {
            let params: AcademicReadParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.as_deref().unwrap_or("").trim().is_empty()
                && params.url.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_read requires identifier or url".into(),
                ));
            }
            validate_academic_parse_options("academic_read", params.parse_options.as_ref())?;
            let output = service
                .academic_read(
                    params.identifier,
                    params.url,
                    params.max_chars,
                    params.output_format,
                    params.parse_options.map(Into::into),
                )
                .await?;
            serialize_output(output, "serialize academic read")
        }
        "academic_parse_pdf" => {
            let params: AcademicParsePdfParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.as_deref().unwrap_or("").trim().is_empty()
                && params.url.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_parse_pdf requires identifier or url".into(),
                ));
            }
            validate_academic_parse_options("academic_parse_pdf", params.parse_options.as_ref())?;
            let output = service
                .academic_parse_pdf(
                    params.identifier,
                    params.url,
                    params.max_chars,
                    params.output_format,
                    params.parse_options.map(Into::into),
                )
                .await?;
            serialize_output(output, "serialize academic parse pdf")
        }
        "academic_download_pdf" => {
            let params: AcademicDownloadPdfParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.as_deref().unwrap_or("").trim().is_empty()
                && params.url.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_download_pdf requires identifier or url".into(),
                ));
            }
            if params.output_path.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_download_pdf.output_path is required".into(),
                ));
            }
            let output = service
                .academic_download_pdf(
                    params.identifier,
                    params.url,
                    params.output_path,
                    params.overwrite.unwrap_or(false),
                )
                .await?;
            serialize_output(output, "serialize academic download pdf")
        }
        "academic_progressive_get" => {
            let params: AcademicProgressiveGetParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.cache_key.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_progressive_get.cache_key is required".into(),
                ));
            }
            if let Some(view) = params.view.as_deref() {
                if !matches!(view, "summary" | "full" | "section") {
                    return Err(GrokSearchError::InvalidParams(
                        "academic_progressive_get.view must be one of summary, full, section"
                            .into(),
                    ));
                }
                if view == "section" && params.section_id.as_deref().unwrap_or("").trim().is_empty()
                {
                    return Err(GrokSearchError::InvalidParams(
                        "academic_progressive_get.section_id is required when view=section".into(),
                    ));
                }
            }
            let output = service.academic_progressive_get(params.into()).await?;
            serialize_output(output, "serialize academic progressive get")
        }
        _ => Err(GrokSearchError::NotFound(format!("unknown tool: {name}"))),
    }
}

pub fn serialize_output<T: serde::Serialize>(output: T, context: &str) -> Result<Value> {
    serde_json::to_value(output).map_err(|err| GrokSearchError::Parse(format!("{context}: {err}")))
}

fn validate_required_query(tool: &str, query: &str) -> Result<()> {
    if query.trim().is_empty() {
        return Err(GrokSearchError::InvalidParams(format!(
            "{tool}.query is required"
        )));
    }
    Ok(())
}

fn validate_range(value: Option<usize>, min: usize, max: usize, name: &str) -> Result<()> {
    if let Some(value) = value {
        if !(min..=max).contains(&value) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{name} must be between {min} and {max}"
            )));
        }
    }
    Ok(())
}

fn validate_pdf_locator(tool: &str, locator: &AcademicPdfLocator) -> Result<()> {
    if locator.is_valid_exactly_one() {
        return Ok(());
    }
    Err(GrokSearchError::InvalidParams(format!(
        "{tool} requires exactly one of identifier, url, or pdf_url"
    )))
}

fn validate_text_processing_mode(tool: &str, mode: Option<&str>) -> Result<()> {
    if let Some(mode) = mode {
        match mode.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "light" | "clean" => {}
            _ => {
                return Err(GrokSearchError::InvalidParams(format!(
                    "{tool}.text_mode must be one of none, light, clean"
                )));
            }
        }
    }
    Ok(())
}

fn validate_structure_view(tool: &str, view: Option<&str>) -> Result<()> {
    if let Some(view) = view {
        if !matches!(view, "summary" | "full" | "section") {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.view must be one of summary, full, section"
            )));
        }
    }
    Ok(())
}

fn validate_academic_parse_options(
    tool: &str,
    options: Option<&AcademicParseOptionsParams>,
) -> Result<()> {
    let Some(options) = options else {
        return Ok(());
    };
    if let Some(mode) = options.text_processing_mode.as_deref() {
        match mode.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "light" | "clean" => {}
            _ => {
                return Err(GrokSearchError::InvalidParams(format!(
                    "{tool}.parse_options.text_processing_mode must be one of none, light, clean"
                )));
            }
        }
    }
    if let Some(llm) = options.llm_progressive.as_ref() {
        if llm.max_chunk_chars == Some(0) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.parse_options.llm_progressive.max_chunk_chars must be greater than 0"
            )));
        }
        if llm.concurrency == Some(0) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.parse_options.llm_progressive.concurrency must be greater than 0"
            )));
        }
        if llm.overlap_chars == Some(0) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.parse_options.llm_progressive.overlap_chars must be greater than 0"
            )));
        }
        if llm.max_output_tokens == Some(0) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.parse_options.llm_progressive.max_output_tokens must be greater than 0"
            )));
        }
        if let Some(input_profile) = llm.input_profile.as_deref() {
            if input_profile != "md_light_plain_refs" {
                return Err(GrokSearchError::InvalidParams(format!(
                    "{tool}.parse_options.llm_progressive.input_profile must be md_light_plain_refs"
                )));
            }
        }
        if let Some(prompt_profile) = llm.prompt_profile.as_deref() {
            if prompt_profile != "compact_v2" {
                return Err(GrokSearchError::InvalidParams(format!(
                    "{tool}.parse_options.llm_progressive.prompt_profile must be compact_v2"
                )));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    pub query: String,
    pub platform: Option<String>,
    pub model: Option<String>,
    pub extra_sources: Option<usize>,
    pub recency_days: Option<u32>,
    #[serde(default)]
    pub include_domains: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
    pub include_content: Option<bool>,
    pub response_format: Option<String>,
}

impl From<WebSearchParams> for WebSearchInput {
    fn from(params: WebSearchParams) -> Self {
        Self {
            query: params.query,
            platform: params.platform,
            model: params.model,
            extra_sources: params.extra_sources,
            recency_days: params.recency_days.filter(|value| *value > 0),
            include_domains: params.include_domains,
            exclude_domains: params.exclude_domains,
            include_content: params.include_content,
            response_format: params.response_format,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSourcesParams {
    pub session_id: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebFetchParams {
    pub url: String,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebMapParams {
    pub url: String,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WechatSearchParams {
    pub query: String,
    pub account: Option<String>,
    pub max_results: Option<usize>,
    pub pages: Option<usize>,
    pub include_content: Option<bool>,
    pub max_content_chars: Option<usize>,
}

impl From<WechatSearchParams> for WechatSearchInput {
    fn from(params: WechatSearchParams) -> Self {
        Self {
            query: params.query,
            account: params.account,
            max_results: params.max_results,
            pages: params.pages,
            include_content: params.include_content,
            max_content_chars: params.max_content_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZhihuSearchParams {
    pub query: String,
    pub count: Option<usize>,
}

impl From<ZhihuSearchParams> for ZhihuSearchInput {
    fn from(params: ZhihuSearchParams) -> Self {
        Self {
            query: params.query,
            count: params.count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DoctorParams {
    pub verbose: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RepoMetadataParams {
    pub url: Option<String>,
    pub provider: Option<RepoProviderParam>,
    pub repo_id: Option<String>,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_type: Option<String>,
    pub include_readme: Option<bool>,
    pub include_card: Option<bool>,
    pub max_text_chars: Option<usize>,
}

impl From<RepoMetadataParams> for RepoMetadataInput {
    fn from(params: RepoMetadataParams) -> Self {
        Self {
            url: params.url,
            provider: params.provider.map(Into::into),
            repo_id: params.repo_id,
            owner: params.owner,
            name: params.name,
            repo_type: params.repo_type,
            include_readme: params.include_readme,
            include_card: params.include_card,
            max_text_chars: params.max_text_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepoProviderParam {
    Github,
    Huggingface,
}

impl From<RepoProviderParam> for RepoProvider {
    fn from(value: RepoProviderParam) -> Self {
        match value {
            RepoProviderParam::Github => RepoProvider::Github,
            RepoProviderParam::Huggingface => RepoProvider::Huggingface,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicSearchParams {
    pub query: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub search_mode: Option<String>,
    pub sort_by: Option<String>,
    pub max_results: Option<usize>,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub open_access_only: Option<bool>,
    pub include_abstract: Option<bool>,
    pub include_citations: Option<bool>,
    pub extract_material_links: Option<bool>,
}

impl From<AcademicSearchParams> for AcademicSearchInput {
    fn from(params: AcademicSearchParams) -> Self {
        Self {
            query: params.query,
            sources: params.sources,
            search_mode: params.search_mode,
            sort_by: params.sort_by,
            max_results: params.max_results,
            year_from: params.year_from,
            year_to: params.year_to,
            open_access_only: params.open_access_only,
            include_abstract: params.include_abstract,
            include_citations: params.include_citations,
            extract_material_links: params.extract_material_links,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicGetParams {
    pub identifier: String,
    pub include_citations: Option<bool>,
    pub include_open_access: Option<bool>,
    pub extract_material_links: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicCitationsParams {
    pub identifier: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicPdfReadParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub text_mode: Option<String>,
    pub max_chars: Option<usize>,
    pub include_raw_content: Option<bool>,
    pub include_processing: Option<bool>,
    pub extract_material_links: Option<bool>,
    pub cache_policy: Option<AcademicPdfCachePolicyParam>,
}

impl From<AcademicPdfReadParams> for AcademicPdfReadInput {
    fn from(params: AcademicPdfReadParams) -> Self {
        Self {
            locator: AcademicPdfLocator {
                identifier: params.identifier,
                url: params.url,
                pdf_url: params.pdf_url,
            },
            text_mode: params.text_mode,
            max_chars: params.max_chars,
            include_raw_content: params.include_raw_content,
            include_processing: params.include_processing,
            extract_material_links: params.extract_material_links,
            cache_policy: params.cache_policy.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicPdfStructureParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub view: Option<String>,
    pub section_id: Option<String>,
    pub profile: Option<AcademicPdfStructureProfileParam>,
    pub model: Option<String>,
    pub cache_policy: Option<AcademicPdfCachePolicyParam>,
    pub include_section_text: Option<bool>,
    pub save_json_path: Option<String>,
    pub max_chars: Option<usize>,
}

impl From<AcademicPdfStructureParams> for AcademicPdfStructureInput {
    fn from(params: AcademicPdfStructureParams) -> Self {
        Self {
            locator: AcademicPdfLocator {
                identifier: params.identifier,
                url: params.url,
                pdf_url: params.pdf_url,
            },
            view: params.view,
            section_id: params.section_id,
            profile: params.profile.map(Into::into),
            model: params.model,
            cache_policy: params.cache_policy.map(Into::into),
            include_section_text: params.include_section_text,
            save_json_path: params.save_json_path,
            max_chars: params.max_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AcademicPdfStructureProfileParam {
    Fast,
    Balanced,
    Strict,
}

impl From<AcademicPdfStructureProfileParam> for AcademicPdfStructureProfile {
    fn from(value: AcademicPdfStructureProfileParam) -> Self {
        match value {
            AcademicPdfStructureProfileParam::Fast => AcademicPdfStructureProfile::Fast,
            AcademicPdfStructureProfileParam::Balanced => AcademicPdfStructureProfile::Balanced,
            AcademicPdfStructureProfileParam::Strict => AcademicPdfStructureProfile::Strict,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AcademicPdfCachePolicyParam {
    Auto,
    Refresh,
    Bypass,
}

impl From<AcademicPdfCachePolicyParam> for AcademicPdfCachePolicy {
    fn from(value: AcademicPdfCachePolicyParam) -> Self {
        match value {
            AcademicPdfCachePolicyParam::Auto => AcademicPdfCachePolicy::Auto,
            AcademicPdfCachePolicyParam::Refresh => AcademicPdfCachePolicy::Refresh,
            AcademicPdfCachePolicyParam::Bypass => AcademicPdfCachePolicy::Bypass,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicPdfArtifactsParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub images_dir: Option<String>,
    pub tables_dir: Option<String>,
    pub extract_images: Option<bool>,
    pub extract_tables: Option<bool>,
    pub text_mode: Option<String>,
    pub max_chars: Option<usize>,
    pub cache_policy: Option<AcademicPdfCachePolicyParam>,
}

impl From<AcademicPdfArtifactsParams> for AcademicPdfArtifactsInput {
    fn from(params: AcademicPdfArtifactsParams) -> Self {
        Self {
            locator: AcademicPdfLocator {
                identifier: params.identifier,
                url: params.url,
                pdf_url: params.pdf_url,
            },
            images_dir: params.images_dir,
            tables_dir: params.tables_dir,
            extract_images: params.extract_images,
            extract_tables: params.extract_tables,
            text_mode: params.text_mode,
            max_chars: params.max_chars,
            cache_policy: params.cache_policy.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicPdfDownloadParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
    pub output_path: String,
    pub overwrite: Option<bool>,
    pub cache_policy: Option<AcademicPdfCachePolicyParam>,
}

impl From<AcademicPdfDownloadParams> for AcademicPdfDownloadInput {
    fn from(params: AcademicPdfDownloadParams) -> Self {
        Self {
            locator: AcademicPdfLocator {
                identifier: params.identifier,
                url: params.url,
                pdf_url: params.pdf_url,
            },
            output_path: params.output_path,
            overwrite: params.overwrite,
            cache_policy: params.cache_policy.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicReadParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub max_chars: Option<usize>,
    pub output_format: Option<String>,
    pub parse_options: Option<AcademicParseOptionsParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicParseOptionsParams {
    pub save_markdown_path: Option<String>,
    pub save_raw_content_path: Option<String>,
    pub images_dir: Option<String>,
    pub tables_dir: Option<String>,
    pub extract_images: Option<bool>,
    pub extract_tables: Option<bool>,
    pub extract_material_links: Option<bool>,
    pub text_processing_mode: Option<String>,
    pub include_raw_content: Option<bool>,
    pub llm_progressive: Option<AcademicLlmProgressiveOptionsParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicLlmProgressiveOptionsParams {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub max_chunk_chars: Option<usize>,
    pub overlap_chars: Option<usize>,
    pub concurrency: Option<usize>,
    pub max_output_tokens: Option<u32>,
    pub input_profile: Option<String>,
    pub prompt_profile: Option<String>,
    pub cache_enabled: Option<bool>,
    pub cache_refresh: Option<bool>,
    pub save_json_path: Option<String>,
    pub include_section_text: Option<bool>,
}

impl From<AcademicParseOptionsParams> for AcademicParseOptions {
    fn from(params: AcademicParseOptionsParams) -> Self {
        Self {
            save_markdown_path: params.save_markdown_path,
            save_raw_content_path: params.save_raw_content_path,
            images_dir: params.images_dir,
            tables_dir: params.tables_dir,
            extract_images: params.extract_images,
            extract_tables: params.extract_tables,
            extract_material_links: params.extract_material_links,
            text_processing_mode: params.text_processing_mode,
            include_raw_content: params.include_raw_content,
            llm_progressive: params.llm_progressive.map(Into::into),
        }
    }
}

impl From<AcademicLlmProgressiveOptionsParams> for AcademicLlmProgressiveOptions {
    fn from(params: AcademicLlmProgressiveOptionsParams) -> Self {
        Self {
            enabled: params.enabled,
            model: params.model,
            max_chunk_chars: params.max_chunk_chars,
            overlap_chars: params.overlap_chars,
            concurrency: params.concurrency,
            max_output_tokens: params.max_output_tokens,
            input_profile: params.input_profile,
            prompt_profile: params.prompt_profile,
            cache_enabled: params.cache_enabled,
            cache_refresh: params.cache_refresh,
            save_json_path: params.save_json_path,
            include_section_text: params.include_section_text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicProgressiveGetParams {
    pub cache_key: String,
    pub view: Option<String>,
    pub section_id: Option<String>,
    pub include_section_text: Option<bool>,
    pub max_chars: Option<usize>,
}

impl From<AcademicProgressiveGetParams> for AcademicProgressiveGetInput {
    fn from(params: AcademicProgressiveGetParams) -> Self {
        Self {
            cache_key: params.cache_key,
            view: params.view,
            section_id: params.section_id,
            include_section_text: params.include_section_text,
            max_chars: params.max_chars,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicParsePdfParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub max_chars: Option<usize>,
    pub output_format: Option<String>,
    pub parse_options: Option<AcademicParseOptionsParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicDownloadPdfParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub output_path: String,
    pub overwrite: Option<bool>,
}

const TOOLS_SPEC_JSON: &str = include_str!("../spec/tools.json");

pub fn tools_list_json() -> Value {
    serde_json::from_str(TOOLS_SPEC_JSON).expect("embedded tools spec JSON must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
