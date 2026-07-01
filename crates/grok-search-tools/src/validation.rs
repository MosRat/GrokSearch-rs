use crate::params::AcademicParseOptionsParams;
use grok_search_types::{AcademicPdfLocator, GrokSearchError, Result};

pub(crate) fn validate_required_query(tool: &str, query: &str) -> Result<()> {
    if query.trim().is_empty() {
        return Err(GrokSearchError::InvalidParams(format!(
            "{tool}.query is required"
        )));
    }
    Ok(())
}

pub(crate) fn validate_range(
    value: Option<usize>,
    min: usize,
    max: usize,
    name: &str,
) -> Result<()> {
    if let Some(value) = value {
        if !(min..=max).contains(&value) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{name} must be between {min} and {max}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_pdf_locator(tool: &str, locator: &AcademicPdfLocator) -> Result<()> {
    if locator.is_valid_exactly_one() {
        return Ok(());
    }
    Err(GrokSearchError::InvalidParams(format!(
        "{tool} requires exactly one of identifier, url, or pdf_url"
    )))
}

pub(crate) fn validate_text_processing_mode(tool: &str, mode: Option<&str>) -> Result<()> {
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

pub(crate) fn validate_structure_view(tool: &str, view: Option<&str>) -> Result<()> {
    if let Some(view) = view {
        if !matches!(view, "summary" | "full" | "section") {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.view must be one of summary, full, section"
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_vision_artifact_options(
    tool: &str,
    profile: Option<&str>,
    max_pages: Option<usize>,
    render_dpi: Option<u16>,
    concurrency: Option<usize>,
) -> Result<()> {
    if let Some(profile) = profile {
        match profile
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .as_str()
        {
            "" | "auto" | "off" | "artifact_micro" => {}
            _ => {
                return Err(GrokSearchError::InvalidParams(format!(
                    "{tool}.vision_profile must be auto, off, or artifact_micro"
                )));
            }
        }
    }
    validate_range(max_pages, 1, 20, &format!("{tool}.vision_max_pages"))?;
    if let Some(render_dpi) = render_dpi {
        if !(50..=100).contains(&render_dpi) {
            return Err(GrokSearchError::InvalidParams(format!(
                "{tool}.vision_render_dpi must be between 50 and 100"
            )));
        }
    }
    validate_range(concurrency, 1, 4, &format!("{tool}.vision_concurrency"))?;
    Ok(())
}

pub(crate) fn validate_academic_parse_options(
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
