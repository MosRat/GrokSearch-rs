use grok_search_config::Config;
use grok_search_types::{
    AcademicLlmProgressiveOptions, AcademicPdfCachePolicy, AcademicPdfStructureInput,
    AcademicPdfStructureProfile,
};

pub(super) fn llm_options_for_structure(
    input: &AcademicPdfStructureInput,
    config: &Config,
) -> AcademicLlmProgressiveOptions {
    let profile = input.profile.unwrap_or_else(|| {
        profile_from_config(&config.progressive_default_profile)
            .unwrap_or(AcademicPdfStructureProfile::Balanced)
    });
    let mut options = match profile {
        AcademicPdfStructureProfile::Fast => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(4_500),
            overlap_chars: Some(300),
            concurrency: Some(3),
            max_output_tokens: Some(1_000),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
        AcademicPdfStructureProfile::Balanced => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(6_500),
            overlap_chars: Some(500),
            concurrency: Some(2),
            max_output_tokens: Some(1_600),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
        AcademicPdfStructureProfile::Strict => AcademicLlmProgressiveOptions {
            max_chunk_chars: Some(5_500),
            overlap_chars: Some(700),
            concurrency: Some(1),
            max_output_tokens: Some(1_800),
            input_profile: Some("md_light_plain_refs".to_string()),
            prompt_profile: Some("compact_v2".to_string()),
            ..Default::default()
        },
    };
    options.enabled = Some(true);
    options.model = input
        .model
        .clone()
        .filter(|model| !model.trim().is_empty())
        .or_else(|| Some(config.progressive_default_model.clone()));
    options.save_json_path = input.save_json_path.clone();
    options.include_section_text = input.include_section_text;
    match input.cache_policy.unwrap_or_default() {
        AcademicPdfCachePolicy::Auto => {
            options.cache_enabled = Some(true);
            options.cache_refresh = Some(false);
        }
        AcademicPdfCachePolicy::Refresh => {
            options.cache_enabled = Some(true);
            options.cache_refresh = Some(true);
        }
        AcademicPdfCachePolicy::Bypass => {
            options.cache_enabled = Some(false);
            options.cache_refresh = Some(false);
        }
    }
    options
}

fn profile_from_config(raw: &str) -> Option<AcademicPdfStructureProfile> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "fast" => Some(AcademicPdfStructureProfile::Fast),
        "" | "balanced" => Some(AcademicPdfStructureProfile::Balanced),
        "strict" => Some(AcademicPdfStructureProfile::Strict),
        _ => None,
    }
}
