use clap::{Args, Subcommand, ValueEnum};
use grok_search_tools::{
    AcademicLlmProgressiveOptionsParams, AcademicParseOptionsParams, AcademicPdfCachePolicyParam,
    AcademicPdfStructureProfileParam,
};

use super::OutputArgs;

#[derive(Debug, Args)]
pub(crate) struct AcademicCommand {
    #[command(subcommand)]
    pub(crate) command: AcademicSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AcademicSubcommand {
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
pub(crate) struct AcademicSearchCommand {
    pub(crate) query: String,
    #[arg(long = "source")]
    pub(crate) sources: Vec<String>,
    #[arg(long)]
    pub(crate) search_mode: Option<String>,
    #[arg(long)]
    pub(crate) sort_by: Option<String>,
    #[arg(long)]
    pub(crate) max_results: Option<usize>,
    #[arg(long)]
    pub(crate) year_from: Option<u32>,
    #[arg(long)]
    pub(crate) year_to: Option<u32>,
    #[arg(long)]
    pub(crate) open_access_only: bool,
    #[arg(long)]
    pub(crate) include_abstract: Option<bool>,
    #[arg(long)]
    pub(crate) include_citations: bool,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicGetCommand {
    pub(crate) identifier: String,
    #[arg(long)]
    pub(crate) include_citations: bool,
    #[arg(long)]
    pub(crate) include_open_access: Option<bool>,
    #[arg(long)]
    pub(crate) extract_material_links: bool,
    #[arg(long, value_enum)]
    pub(crate) cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicCitationsCommand {
    pub(crate) identifier: String,
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args, Default)]
pub(crate) struct AcademicPdfLocatorArgs {
    #[arg(long)]
    pub(crate) identifier: Option<String>,
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) pdf_url: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicPdfReadCommand {
    #[command(flatten)]
    pub(crate) locator: AcademicPdfLocatorArgs,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    pub(crate) text_mode: Option<String>,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[arg(long)]
    pub(crate) include_raw_content: bool,
    #[arg(long)]
    pub(crate) include_processing: bool,
    #[arg(long)]
    pub(crate) extract_material_links: bool,
    #[arg(long, value_enum)]
    pub(crate) cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicPdfStructureCommand {
    #[command(flatten)]
    pub(crate) locator: AcademicPdfLocatorArgs,
    #[arg(long, value_parser = ["summary", "full", "section"])]
    pub(crate) view: Option<String>,
    #[arg(long)]
    pub(crate) section_id: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) profile: Option<AcademicPdfStructureProfileArg>,
    #[arg(long)]
    pub(crate) model: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[arg(long)]
    pub(crate) include_section_text: bool,
    #[arg(long)]
    pub(crate) save_json_path: Option<String>,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum AcademicPdfStructureProfileArg {
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
pub(crate) enum AcademicPdfCachePolicyArg {
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum AcademicPdfVisionProfileArg {
    Auto,
    Off,
    ArtifactMicro,
}

impl AcademicPdfVisionProfileArg {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AcademicPdfVisionProfileArg::Auto => "auto",
            AcademicPdfVisionProfileArg::Off => "off",
            AcademicPdfVisionProfileArg::ArtifactMicro => "artifact_micro",
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct AcademicPdfArtifactsCommand {
    #[command(flatten)]
    pub(crate) locator: AcademicPdfLocatorArgs,
    #[arg(long)]
    pub(crate) images_dir: Option<String>,
    #[arg(long)]
    pub(crate) tables_dir: Option<String>,
    #[arg(long)]
    pub(crate) extract_images: bool,
    #[arg(long)]
    pub(crate) extract_tables: bool,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    pub(crate) text_mode: Option<String>,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[arg(long, value_enum)]
    pub(crate) cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[arg(long, value_enum)]
    pub(crate) vision_profile: Option<AcademicPdfVisionProfileArg>,
    #[arg(long)]
    pub(crate) vision_max_pages: Option<usize>,
    #[arg(long)]
    pub(crate) vision_render_dpi: Option<u16>,
    #[arg(long)]
    pub(crate) vision_concurrency: Option<usize>,
    #[arg(long, value_enum)]
    pub(crate) vision_cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[arg(long)]
    pub(crate) vision_dir: Option<String>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicPdfDownloadCommand {
    #[command(flatten)]
    pub(crate) locator: AcademicPdfLocatorArgs,
    #[arg(long)]
    pub(crate) output_path: String,
    #[arg(long)]
    pub(crate) overwrite: bool,
    #[arg(long, value_enum)]
    pub(crate) cache_policy: Option<AcademicPdfCachePolicyArg>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicReadCommand {
    #[arg(long)]
    pub(crate) identifier: Option<String>,
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[arg(long)]
    pub(crate) output_format: Option<String>,
    #[command(flatten)]
    pub(crate) parse: AcademicParseOptionsCommand,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicParsePdfCommand {
    #[arg(long)]
    pub(crate) identifier: Option<String>,
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[arg(long)]
    pub(crate) output_format: Option<String>,
    #[command(flatten)]
    pub(crate) parse: AcademicParseOptionsCommand,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicDownloadPdfCommand {
    #[arg(long)]
    pub(crate) identifier: Option<String>,
    #[arg(long)]
    pub(crate) url: Option<String>,
    #[arg(long)]
    pub(crate) output_path: String,
    #[arg(long)]
    pub(crate) overwrite: bool,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AcademicProgressiveGetCommand {
    pub(crate) cache_key: String,
    #[arg(long, value_parser = ["summary", "full", "section"])]
    pub(crate) view: Option<String>,
    #[arg(long)]
    pub(crate) section_id: Option<String>,
    #[arg(long)]
    pub(crate) include_section_text: bool,
    #[arg(long)]
    pub(crate) max_chars: Option<usize>,
    #[command(flatten)]
    pub(crate) output: OutputArgs,
}

#[derive(Debug, Args, Default)]
pub(crate) struct AcademicParseOptionsCommand {
    #[arg(long)]
    pub(crate) save_markdown_path: Option<String>,
    #[arg(long)]
    pub(crate) save_raw_content_path: Option<String>,
    #[arg(long)]
    pub(crate) images_dir: Option<String>,
    #[arg(long)]
    pub(crate) tables_dir: Option<String>,
    #[arg(long)]
    pub(crate) extract_images: bool,
    #[arg(long)]
    pub(crate) extract_tables: bool,
    #[arg(long)]
    pub(crate) extract_material_links: bool,
    #[arg(long, value_parser = ["none", "light", "clean"])]
    pub(crate) text_processing_mode: Option<String>,
    #[arg(long)]
    pub(crate) include_raw_content: bool,
    #[arg(long)]
    pub(crate) llm_progressive: bool,
    #[arg(long)]
    pub(crate) llm_progressive_model: Option<String>,
    #[arg(long)]
    pub(crate) llm_progressive_max_chunk_chars: Option<usize>,
    #[arg(long)]
    pub(crate) llm_progressive_overlap_chars: Option<usize>,
    #[arg(long)]
    pub(crate) llm_progressive_concurrency: Option<usize>,
    #[arg(long)]
    pub(crate) llm_progressive_max_output_tokens: Option<u32>,
    #[arg(long, value_parser = ["md_light_plain_refs"])]
    pub(crate) llm_progressive_input_profile: Option<String>,
    #[arg(long, value_parser = ["compact_v2"])]
    pub(crate) llm_progressive_prompt_profile: Option<String>,
    #[arg(long)]
    pub(crate) llm_progressive_cache_enabled: Option<bool>,
    #[arg(long)]
    pub(crate) llm_progressive_cache_refresh: bool,
    #[arg(long)]
    pub(crate) llm_progressive_save_json_path: Option<String>,
    #[arg(long)]
    pub(crate) llm_progressive_include_section_text: bool,
}

impl AcademicParseOptionsCommand {
    pub(crate) fn into_options(self) -> Option<AcademicParseOptionsParams> {
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
