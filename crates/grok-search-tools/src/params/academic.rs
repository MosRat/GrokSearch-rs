use grok_search_types::{
    AcademicLlmProgressiveOptions, AcademicParseOptions, AcademicPdfArtifactsInput,
    AcademicPdfCachePolicy, AcademicPdfDownloadInput, AcademicPdfLocator, AcademicPdfReadInput,
    AcademicPdfStructureInput, AcademicPdfStructureProfile, AcademicProgressiveGetInput,
    AcademicSearchInput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    pub vision_profile: Option<String>,
    pub vision_max_pages: Option<usize>,
    pub vision_render_dpi: Option<u16>,
    pub vision_concurrency: Option<usize>,
    pub vision_cache_policy: Option<AcademicPdfCachePolicyParam>,
    pub vision_dir: Option<String>,
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
            vision_profile: params.vision_profile,
            vision_max_pages: params.vision_max_pages,
            vision_render_dpi: params.vision_render_dpi,
            vision_concurrency: params.vision_concurrency,
            vision_cache_policy: params.vision_cache_policy.map(Into::into),
            vision_dir: params.vision_dir,
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
            vision_profile: None,
            vision_max_pages: None,
            vision_render_dpi: None,
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
