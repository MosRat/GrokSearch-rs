use crate::params::{
    AcademicCitationsParams, AcademicDownloadPdfParams, AcademicGetParams, AcademicParsePdfParams,
    AcademicPdfArtifactsParams, AcademicPdfDownloadParams, AcademicPdfReadParams,
    AcademicPdfStructureParams, AcademicProgressiveGetParams, AcademicReadParams,
    AcademicSearchParams, DoctorParams, GetSourcesParams, RepoMetadataParams, WebFetchParams,
    WebMapParams, WebSearchParams, WechatSearchParams, ZhihuSearchParams,
};
use crate::validation::{
    validate_academic_parse_options, validate_pdf_locator, validate_range, validate_required_query,
    validate_structure_view, validate_text_processing_mode, validate_vision_artifact_options,
};
use grok_search_service::SearchService;
use grok_search_types::{
    AcademicPdfArtifactsInput, AcademicPdfDownloadInput, AcademicPdfReadInput,
    AcademicPdfStructureInput, GrokSearchError, Result,
};
use serde_json::{json, Value};

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
            validate_vision_artifact_options(
                "academic_pdf_artifacts",
                input.vision_profile.as_deref(),
                input.vision_max_pages,
                input.vision_render_dpi,
                input.vision_concurrency,
            )?;
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
