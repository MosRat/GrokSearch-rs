use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use grok_search_cache::{RedbVisionCache, VisionCachePut, VisionCacheStore};
use grok_search_config::Config;
use grok_search_llm::{LlmClient, LlmContentBlock, LlmMessage, LlmRequest, LlmRole};
use grok_search_pdf::{select_vision_pages, ParsedPdfDetails, PdfRenderedPage, PdfVisionPage};
use grok_search_types::{
    AcademicPdfArtifactsInput, AcademicPdfCachePolicy, AcademicPdfFigureCompletion,
    AcademicPdfFigureRepair, AcademicPdfTableCompletion, AcademicPdfTableRepair,
    AcademicPdfVisionArtifacts, AcademicPdfVisionCacheInfo, AcademicPdfVisionCallReport,
    AcademicPdfVisionRawCompletion, AcademicPdfVisionValidation, AcademicPdfVisualObject,
    GrokSearchError, Result,
};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::llm_progressive;

mod artifact_writer;
mod prompt;

pub(crate) use artifact_writer::write_refined_completion_artifacts;
use artifact_writer::write_vision_artifact;
use prompt::{page_prompt, system_prompt};

const PROFILE_OFF: &str = "off";
const PROFILE_AUTO: &str = "auto";
const PROFILE_ARTIFACT_MICRO: &str = "artifact_micro";
const PROMPT_VERSION: &str = "artifact_completion_v2";
const DEFAULT_RENDER_DPI: u16 = 65;
const DEFAULT_MAX_PAGES: usize = 8;
const DEFAULT_CONCURRENCY: usize = 4;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_200;

#[derive(Debug, Clone)]
pub(crate) struct ArtifactVisionRunConfig {
    pub profile: String,
    pub model: String,
    pub max_pages: usize,
    pub render_dpi: u16,
    pub concurrency: usize,
    pub cache_policy: AcademicPdfCachePolicy,
    pub vision_dir: Option<String>,
}

impl ArtifactVisionRunConfig {
    pub fn from_input(input: &AcademicPdfArtifactsInput, config: &Config) -> Result<Self> {
        let profile = input
            .vision_profile
            .clone()
            .unwrap_or_else(|| PROFILE_AUTO.to_string());
        let normalized_profile = normalize_profile(&profile, config)?;
        let max_pages = input.vision_max_pages.unwrap_or(DEFAULT_MAX_PAGES);
        if !(1..=20).contains(&max_pages) {
            return Err(GrokSearchError::InvalidParams(
                "academic_pdf_artifacts.vision_max_pages must be between 1 and 20".to_string(),
            ));
        }
        let render_dpi = input.vision_render_dpi.unwrap_or(DEFAULT_RENDER_DPI);
        if !(50..=100).contains(&render_dpi) {
            return Err(GrokSearchError::InvalidParams(
                "academic_pdf_artifacts.vision_render_dpi must be between 50 and 100".to_string(),
            ));
        }
        let concurrency = input.vision_concurrency.unwrap_or(DEFAULT_CONCURRENCY);
        if !(1..=4).contains(&concurrency) {
            return Err(GrokSearchError::InvalidParams(
                "academic_pdf_artifacts.vision_concurrency must be between 1 and 4".to_string(),
            ));
        }
        Ok(Self {
            profile: normalized_profile,
            model: config
                .progressive_default_model
                .trim()
                .is_empty()
                .then(|| "MiniMax-M3".to_string())
                .unwrap_or_else(|| config.progressive_default_model.clone()),
            max_pages,
            render_dpi,
            concurrency,
            cache_policy: input
                .vision_cache_policy
                .unwrap_or(AcademicPdfCachePolicy::Auto),
            vision_dir: input.vision_dir.clone(),
        })
    }

    pub fn enabled(&self) -> bool {
        self.profile == PROFILE_ARTIFACT_MICRO
    }
}

#[derive(Debug, Clone)]
struct VisionPageJob {
    source: PdfVisionPage,
    rendered: PdfRenderedPage,
}

#[derive(Debug, Clone)]
struct VisionPageOutcome {
    report: AcademicPdfVisionCallReport,
    visual_objects: Vec<AcademicPdfVisualObject>,
    table_repairs: Vec<AcademicPdfTableRepair>,
    figure_repairs: Vec<AcademicPdfFigureRepair>,
    figure_completions: Vec<AcademicPdfFigureCompletion>,
    table_completions: Vec<AcademicPdfTableCompletion>,
    raw_completions: Vec<AcademicPdfVisionRawCompletion>,
    layout_warnings: Vec<String>,
}

#[derive(Debug)]
struct LlmAttemptOutcome {
    response: grok_search_llm::LlmResponse,
    attempts: u32,
    backoff_ms: u64,
}

pub(crate) async fn run_artifact_micro(
    parsed: &ParsedPdfDetails,
    input: &AcademicPdfArtifactsInput,
    config: &Config,
    http: &reqwest::Client,
) -> Result<Option<AcademicPdfVisionArtifacts>> {
    let run_config = ArtifactVisionRunConfig::from_input(input, config)?;
    if !run_config.enabled() {
        return Ok(None);
    }
    let mut output = match run_inner(parsed, &run_config, config, http).await {
        Ok(output) => output,
        Err(err) => AcademicPdfVisionArtifacts {
            profile: run_config.profile.clone(),
            status: "failed_nonfatal".to_string(),
            model: Some(run_config.model.clone()),
            pages_analyzed: 0,
            pages_considered: parsed
                .vision_source
                .as_ref()
                .map(|source| source.pages.len())
                .unwrap_or(0),
            render_dpi: run_config.render_dpi,
            cache: None,
            visual_objects: Vec::new(),
            table_repairs: Vec::new(),
            figure_repairs: Vec::new(),
            figure_completions: Vec::new(),
            table_completions: Vec::new(),
            raw_completions: Vec::new(),
            layout_warnings: Vec::new(),
            calls: Vec::new(),
            warnings: vec![err.to_string()],
        },
    };
    if let Some(dir) = run_config.vision_dir.as_deref() {
        match write_vision_artifact(dir, &output) {
            Ok(path) => output
                .warnings
                .push(format!("vision diagnostics written to {path}")),
            Err(err) => output
                .warnings
                .push(format!("write vision diagnostics failed: {err}")),
        }
    }
    Ok(Some(output))
}

async fn run_inner(
    parsed: &ParsedPdfDetails,
    run_config: &ArtifactVisionRunConfig,
    config: &Config,
    http: &reqwest::Client,
) -> Result<AcademicPdfVisionArtifacts> {
    let Some(source) = parsed.vision_source.as_ref() else {
        return Ok(skipped_output(
            run_config,
            0,
            "vision_source_unavailable; parse with vision_profile=artifact_micro",
        ));
    };
    let selected_pages = select_vision_pages(source, run_config.max_pages);
    let page_text_sha256 = sha256_hex(selected_pages_text(&selected_pages).as_bytes());
    let strategy_hash =
        vision_strategy_hash(parsed, &selected_pages, &page_text_sha256, run_config);
    let cache_key = format!("vision:v1:{strategy_hash}");
    let mut cache_info = AcademicPdfVisionCacheInfo {
        key: cache_key.clone(),
        hit: false,
        stored: false,
        warnings: Vec::new(),
    };
    let cache_allowed = config.progressive_cache_enabled
        && !matches!(run_config.cache_policy, AcademicPdfCachePolicy::Bypass);
    if cache_allowed && !matches!(run_config.cache_policy, AcademicPdfCachePolicy::Refresh) {
        match RedbVisionCache::open(&config.progressive_cache_path) {
            Ok(cache) => match cache.get(&cache_key) {
                Ok(Some(entry)) => {
                    let mut cached =
                        serde_json::from_slice::<AcademicPdfVisionArtifacts>(&entry.bytes)
                            .map_err(|err| {
                                GrokSearchError::Parse(format!(
                                    "parse cached vision artifact JSON: {err}"
                                ))
                            })?;
                    cache_info.hit = true;
                    cache_info.stored = true;
                    cached.cache = Some(cache_info);
                    cached.status = "cache_hit".to_string();
                    return Ok(cached);
                }
                Ok(None) => {}
                Err(err) => cache_info
                    .warnings
                    .push(format!("vision cache read failed: {err}")),
            },
            Err(err) => cache_info
                .warnings
                .push(format!("vision cache open failed: {err}")),
        }
    }

    if selected_pages.is_empty() {
        let mut output = skipped_output(run_config, source.pages.len(), "no_triage_pages_selected");
        output.cache = Some(cache_info);
        return Ok(output);
    }

    let Some(render) = parsed.vision_render.as_ref() else {
        let mut output = skipped_output(
            run_config,
            source.pages.len(),
            "vision_render_unavailable; pdf pipeline did not render selected pages",
        );
        output.cache = Some(cache_info);
        return Ok(output);
    };
    let rendered_by_page = render
        .pages
        .iter()
        .cloned()
        .map(|page| (page.page_number, page))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut warnings = source.warnings.clone();
    warnings.extend(
        render
            .failures
            .iter()
            .map(|failure| failure.warning.clone()),
    );
    let jobs = selected_pages
        .iter()
        .filter_map(|page| {
            rendered_by_page
                .get(&page.page_number)
                .map(|rendered| VisionPageJob {
                    source: page.clone(),
                    rendered: rendered.clone(),
                })
        })
        .collect::<Vec<_>>();
    if jobs.is_empty() {
        let mut output = skipped_output(run_config, source.pages.len(), "no_pages_rendered");
        output.warnings.extend(warnings);
        output.cache = Some(cache_info);
        return Ok(output);
    }

    let client = Arc::new(llm_progressive::build_client(config, http)?);
    let semaphore = Arc::new(Semaphore::new(run_config.concurrency));
    let mut handles = Vec::new();
    for job in jobs.clone() {
        let permitter = semaphore.clone();
        let client = client.clone();
        let run_config = run_config.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permitter.acquire_owned().await.map_err(|err| {
                GrokSearchError::Provider(format!("artifact_micro semaphore closed: {err}"))
            })?;
            process_page(client.as_ref(), &run_config, job).await
        }));
    }

    let mut output = AcademicPdfVisionArtifacts {
        profile: run_config.profile.clone(),
        status: "ok".to_string(),
        model: Some(run_config.model.clone()),
        pages_analyzed: 0,
        pages_considered: source.pages.len(),
        render_dpi: run_config.render_dpi,
        cache: Some(cache_info.clone()),
        visual_objects: Vec::new(),
        table_repairs: Vec::new(),
        figure_repairs: Vec::new(),
        figure_completions: Vec::new(),
        table_completions: Vec::new(),
        raw_completions: Vec::new(),
        layout_warnings: Vec::new(),
        calls: Vec::new(),
        warnings,
    };

    for handle in handles {
        match handle.await {
            Ok(Ok(result)) => merge_page_result(&mut output, result),
            Ok(Err(err)) => {
                output.status = "partial".to_string();
                output.warnings.push(err.to_string());
            }
            Err(err) => {
                output.status = "partial".to_string();
                output
                    .warnings
                    .push(format!("artifact_micro task join failed: {err}"));
            }
        }
    }
    output.pages_analyzed = output
        .calls
        .iter()
        .filter(|call| call.status == "ok" || call.status == "partial")
        .count();

    if cache_allowed {
        match serde_json::to_vec(&output) {
            Ok(bytes) => match RedbVisionCache::open(&config.progressive_cache_path) {
                Ok(cache) => {
                    let put = VisionCachePut {
                        cache_key: cache_key.clone(),
                        bytes,
                        ttl_seconds: Some(config.progressive_cache_ttl_seconds),
                        pdf_sha256: parsed.pdf_sha256.clone(),
                        page_text_sha256,
                        strategy_hash,
                        model: run_config.model.clone(),
                        profile: run_config.profile.clone(),
                        render_dpi: run_config.render_dpi,
                    };
                    match cache.put(put, config.progressive_cache_max_entries) {
                        Ok(_) => {
                            if let Some(cache) = output.cache.as_mut() {
                                cache.stored = true;
                            }
                        }
                        Err(err) => {
                            if let Some(cache) = output.cache.as_mut() {
                                cache
                                    .warnings
                                    .push(format!("vision cache write failed: {err}"));
                            }
                        }
                    }
                }
                Err(err) => {
                    if let Some(cache) = output.cache.as_mut() {
                        cache
                            .warnings
                            .push(format!("vision cache open failed: {err}"));
                    }
                }
            },
            Err(err) => output
                .warnings
                .push(format!("serialize vision artifact JSON failed: {err}")),
        }
    }
    Ok(output)
}

async fn process_page(
    client: &grok_search_llm::AnthropicMessagesClient,
    run_config: &ArtifactVisionRunConfig,
    job: VisionPageJob,
) -> Result<VisionPageOutcome> {
    let started = Instant::now();
    let prompt = page_prompt(&job.source, &job.rendered);
    let image_data = base64::engine::general_purpose::STANDARD.encode(&job.rendered.png);
    let request = LlmRequest {
        model: run_config.model.clone(),
        messages: vec![LlmMessage::new(
            LlmRole::User,
            vec![
                LlmContentBlock::text(prompt),
                LlmContentBlock::image_base64("image/png", image_data, Some("low".to_string())),
            ],
        )],
        system: Some(system_prompt().to_string()),
        max_tokens: Some(DEFAULT_MAX_OUTPUT_TOKENS),
        temperature: Some(0.0),
        top_p: None,
        tools: Vec::new(),
        tool_choice: None,
        stop: Vec::new(),
        metadata: None,
    };
    let response = complete_with_backoff(client, request).await?;
    let text = response
        .response
        .content
        .iter()
        .find_map(|block| block.as_text())
        .unwrap_or_default()
        .to_string()
        .trim()
        .to_string();
    match parse_page_output(job.source.page_number, &text, false) {
        Ok(mut outcome) => {
            outcome.report.elapsed_ms = Some(started.elapsed().as_millis() as u64);
            outcome.report.attempts = response.attempts;
            outcome.report.backoff_ms = response.backoff_ms;
            outcome.report.priority = job.source.triage_priority;
            outcome.report.reasons = job.source.triage_reasons.clone();
            Ok(outcome)
        }
        Err(first_err) => {
            let repair = repair_json(client, run_config, &text).await?;
            let repaired_text = repair
                .response
                .content
                .iter()
                .find_map(|block| block.as_text())
                .unwrap_or_default()
                .to_string();
            let mut outcome = parse_page_output(job.source.page_number, &repaired_text, true)?;
            outcome.report.elapsed_ms = Some(started.elapsed().as_millis() as u64);
            outcome.report.attempts = response.attempts.saturating_add(repair.attempts);
            outcome.report.backoff_ms = response.backoff_ms.saturating_add(repair.backoff_ms);
            outcome.report.priority = job.source.triage_priority;
            outcome.report.reasons = job.source.triage_reasons.clone();
            outcome
                .report
                .warnings
                .push(format!("json_repaired_after: {first_err}"));
            Ok(outcome)
        }
    }
}

async fn repair_json(
    client: &grok_search_llm::AnthropicMessagesClient,
    run_config: &ArtifactVisionRunConfig,
    raw: &str,
) -> Result<LlmAttemptOutcome> {
    let request = LlmRequest {
        model: run_config.model.clone(),
        messages: vec![LlmMessage::text(
            LlmRole::User,
            format!(
                "Repair this artifact_micro response into strict JSON only. Preserve only allowed fields. Input:\n{raw}"
            ),
        )],
        system: Some(system_prompt().to_string()),
        max_tokens: Some(DEFAULT_MAX_OUTPUT_TOKENS),
        temperature: Some(0.0),
        top_p: None,
        tools: Vec::new(),
        tool_choice: None,
        stop: Vec::new(),
        metadata: None,
    };
    complete_with_backoff(client, request).await
}

fn parse_page_output(page: usize, raw: &str, repaired: bool) -> Result<VisionPageOutcome> {
    let value: Value = serde_json::from_str(raw)
        .or_else(|_| serde_json::from_str(&extract_json_object(raw)))
        .map_err(|err| {
            GrokSearchError::Parse(format!("artifact_micro JSON parse failed: {err}"))
        })?;
    let mut report = AcademicPdfVisionCallReport {
        page,
        status: "ok".to_string(),
        priority: 0,
        reasons: Vec::new(),
        elapsed_ms: None,
        attempts: 1,
        backoff_ms: 0,
        json_valid: !repaired,
        repaired,
        warnings: Vec::new(),
    };
    if value.get("patches").is_some() {
        report
            .warnings
            .push("discarded_model_text_patches".to_string());
    }
    let visual_objects = normalize_visual_objects(page, value.get("visual_objects"));
    let mut figure_completions =
        normalize_figure_completions(page, value.get("figure_completions"));
    let mut table_completions = normalize_table_completions(page, value.get("table_completions"));
    let raw_completions = raw_completion_summaries(page, &figure_completions, &table_completions);
    for completion in &mut figure_completions {
        refine_figure_completion(completion);
    }
    for completion in &mut table_completions {
        refine_table_completion(completion);
    }
    let mut table_repairs = normalize_table_repairs(page, value.get("table_repairs"));
    let mut figure_repairs = normalize_figure_repairs(page, value.get("figure_repairs"));
    table_repairs.extend(table_completions.iter().map(table_repair_from_completion));
    figure_repairs.extend(figure_completions.iter().map(figure_repair_from_completion));
    let layout_warnings = normalize_warnings(value.get("layout_warnings"));
    Ok(VisionPageOutcome {
        report,
        visual_objects,
        table_repairs,
        figure_repairs,
        figure_completions,
        table_completions,
        raw_completions,
        layout_warnings,
    })
}

fn normalize_visual_objects(page: usize, value: Option<&Value>) -> Vec<AcademicPdfVisualObject> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(3)
        .map(|item| AcademicPdfVisualObject {
            page: item
                .get("page")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(page),
            kind: normalize_kind(string_field(item, "kind").unwrap_or_else(|| "unknown".into())),
            label: string_field(item, "label"),
            summary: string_field(item, "summary").map(limit_short),
            confidence: string_field(item, "confidence"),
        })
        .collect()
}

fn normalize_table_repairs(page: usize, value: Option<&Value>) -> Vec<AcademicPdfTableRepair> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1)
        .map(|item| AcademicPdfTableRepair {
            page: item
                .get("page")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(page),
            label: string_field(item, "label"),
            status: normalize_status(string_field(item, "status").unwrap_or_default()),
            header: list_of_strings(item.get("header"))
                .into_iter()
                .take(5)
                .collect(),
            sample_rows: item
                .get("sample_rows")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(1)
                .map(|row| list_of_strings(Some(row)).into_iter().take(5).collect())
                .collect(),
            notes: string_field(item, "notes").map(limit_short),
        })
        .collect()
}

fn normalize_figure_repairs(page: usize, value: Option<&Value>) -> Vec<AcademicPdfFigureRepair> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(1)
        .map(|item| AcademicPdfFigureRepair {
            page: item
                .get("page")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(page),
            label: string_field(item, "label"),
            status: normalize_status(string_field(item, "status").unwrap_or_default()),
            notes: string_field(item, "notes").map(limit_short),
        })
        .collect()
}

fn normalize_figure_completions(
    page: usize,
    value: Option<&Value>,
) -> Vec<AcademicPdfFigureCompletion> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .map(|item| AcademicPdfFigureCompletion {
            page: item
                .get("page")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(page),
            label: string_field(item, "label"),
            caption: string_field(item, "caption").map(limit_medium),
            bbox_norm: bbox_field(item, "bbox_norm"),
            caption_bbox_norm: bbox_field(item, "caption_bbox_norm"),
            status: normalize_figure_completion_status(
                string_field(item, "status").unwrap_or_default(),
            ),
            confidence: number_field(item, "confidence"),
            notes: string_field(item, "notes").map(limit_medium),
            refined_bbox_norm: Vec::new(),
            crop_path: None,
            validation: None,
            warnings: Vec::new(),
            raw: Some(safe_raw_json(item)),
        })
        .collect()
}

fn normalize_table_completions(
    page: usize,
    value: Option<&Value>,
) -> Vec<AcademicPdfTableCompletion> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .map(|item| {
            let headers = list_of_strings(item.get("headers"));
            let rows = list_of_rows(item.get("rows"), 20, 12);
            let markdown = string_field(item, "markdown")
                .filter(|text| text.contains('|'))
                .or_else(|| synthesize_markdown(&headers, &rows));
            AcademicPdfTableCompletion {
                page: item
                    .get("page")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(page),
                label: string_field(item, "label"),
                caption: string_field(item, "caption").map(limit_medium),
                bbox_norm: bbox_field(item, "bbox_norm"),
                status: normalize_table_completion_status(
                    string_field(item, "status").unwrap_or_default(),
                ),
                headers,
                rows,
                markdown,
                confidence: number_field(item, "confidence"),
                notes: string_field(item, "notes").map(limit_medium),
                refined_bbox_norm: Vec::new(),
                crop_path: None,
                markdown_path: None,
                validation: None,
                warnings: Vec::new(),
                raw: Some(safe_raw_json(item)),
            }
        })
        .collect()
}

fn refine_figure_completion(completion: &mut AcademicPdfFigureCompletion) {
    let mut warnings = Vec::new();
    if completion.status == "not_visible" {
        completion.validation = Some(validation("ok", warnings));
        return;
    }
    let Some(mut bbox) = validate_bbox(&completion.bbox_norm) else {
        warnings.push("invalid_or_missing_figure_bbox".to_string());
        completion.validation = Some(validation("warning", warnings.clone()));
        completion.warnings.extend(warnings);
        return;
    };
    let mut max_bottom = 1.0f32;
    let mut min_top = 0.0f32;
    if let Some(caption_bbox) = validate_bbox(&completion.caption_bbox_norm) {
        if bbox_overlaps_or_touches(&bbox, &caption_bbox) {
            if caption_bbox[1] >= bbox[1] && caption_bbox[1] <= bbox[3] {
                bbox[3] = (caption_bbox[1] - 0.005).max(bbox[1] + 0.02);
                max_bottom = bbox[3];
                warnings.push("refined_figure_bbox_removed_caption_overlap".to_string());
            } else if caption_bbox[3] >= bbox[1] && caption_bbox[3] <= bbox[3] {
                bbox[1] = (caption_bbox[3] + 0.005).min(bbox[3] - 0.02);
                min_top = bbox[1];
                warnings.push("refined_figure_bbox_removed_caption_overlap".to_string());
            }
        }
    }
    bbox = expand_bbox(bbox, 0.015, 0.01);
    bbox[1] = bbox[1].max(min_top);
    bbox[3] = bbox[3].min(max_bottom);
    if bbox_area(&bbox) < 0.004 {
        warnings.push("refined_figure_bbox_too_small".to_string());
    }
    completion.refined_bbox_norm = bbox.to_vec();
    let status = if warnings.iter().any(|item| item.contains("too_small")) {
        "warning"
    } else {
        "ok"
    };
    completion.validation = Some(validation(status, warnings.clone()));
    completion.warnings.extend(warnings);
}

fn refine_table_completion(completion: &mut AcademicPdfTableCompletion) {
    let mut warnings = Vec::new();
    if completion.status == "not_visible" {
        completion.validation = Some(validation("ok", warnings));
        return;
    }
    if let Some(bbox) = validate_bbox(&completion.bbox_norm) {
        completion.refined_bbox_norm = expand_bbox(bbox, 0.025, 0.015).to_vec();
    } else {
        warnings.push("invalid_or_missing_table_bbox".to_string());
    }
    if completion.markdown.is_none() {
        completion.markdown = synthesize_markdown(&completion.headers, &completion.rows);
    }
    if completion.headers.is_empty() && completion.rows.is_empty() {
        warnings.push("table_completion_has_no_cells".to_string());
    }
    if let Some(expected) = expected_table_width(&completion.headers, &completion.rows) {
        if completion
            .rows
            .iter()
            .any(|row| !row.is_empty() && row.len() != expected)
        {
            warnings.push("table_row_width_mismatch".to_string());
        }
    }
    if completion
        .rows
        .iter()
        .flatten()
        .any(|cell| flattened_exponent_like(cell))
    {
        warnings.push("table_values_may_need_superscript_or_exponent_validation".to_string());
    }
    let status = if warnings.is_empty() { "ok" } else { "warning" };
    completion.validation = Some(validation(status, warnings.clone()));
    completion.warnings.extend(warnings);
}

fn normalize_warnings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(3)
        .filter_map(|item| match item {
            Value::String(text) => Some(text.clone()),
            Value::Object(_) => Some(item.to_string()),
            _ => None,
        })
        .map(limit_short)
        .collect()
}

fn list_of_strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect(),
        Some(Value::String(text)) => text
            .split('|')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn list_of_rows(value: Option<&Value>, max_rows: usize, max_cols: usize) -> Vec<Vec<String>> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(max_rows)
        .map(|row| {
            list_of_strings(Some(row))
                .into_iter()
                .take(max_cols)
                .collect()
        })
        .filter(|row: &Vec<String>| !row.is_empty())
        .collect()
}

fn string_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn number_field(value: &Value, name: &str) -> Option<f32> {
    value.get(name).and_then(|value| match value {
        Value::Number(number) => number.as_f64().map(|value| value as f32),
        Value::String(text) => text.parse::<f32>().ok(),
        _ => None,
    })
}

fn bbox_field(value: &Value, name: &str) -> Vec<f32> {
    value
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .filter_map(|item| match item {
            Value::Number(number) => number.as_f64().map(|value| value as f32),
            Value::String(text) => text.parse::<f32>().ok(),
            _ => None,
        })
        .collect()
}

fn normalize_kind(kind: String) -> String {
    match kind.trim().to_ascii_lowercase().as_str() {
        "figure" | "table" | "chart" | "diagram" | "resource" | "equation" => kind,
        "section" | "text" | "code" | "dataset" => "resource".to_string(),
        _ => "unknown".to_string(),
    }
}

fn normalize_status(status: String) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "ok" | "found" | "present" => "ok".to_string(),
        "missing" | "not_found" | "missed" => "missed".to_string(),
        "uncertain" | "partial" => "uncertain".to_string(),
        _ => "uncertain".to_string(),
    }
}

fn normalize_figure_completion_status(status: String) -> String {
    match status
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "crop_ready" | "visible" | "ready" | "usable" | "ok" => "crop_ready".to_string(),
        "fragmented" | "partial" | "broken" => "fragmented".to_string(),
        "not_visible" | "not visible" | "missing" | "not_found" => "not_visible".to_string(),
        _ => "uncertain".to_string(),
    }
}

fn normalize_table_completion_status(status: String) -> String {
    match status
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "reconstructed" | "complete" | "visible" | "usable" | "ok" => "reconstructed".to_string(),
        "partial" | "fragmented" | "missed" => "partial".to_string(),
        "not_visible" | "not visible" | "missing" | "not_found" => "not_visible".to_string(),
        _ => "uncertain".to_string(),
    }
}

fn limit_short(text: String) -> String {
    text.chars().take(180).collect()
}

fn limit_medium(text: String) -> String {
    text.chars().take(600).collect()
}

fn safe_raw_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for key in [
                "page",
                "label",
                "caption",
                "bbox_norm",
                "caption_bbox_norm",
                "status",
                "headers",
                "rows",
                "markdown",
                "confidence",
                "notes",
            ] {
                if let Some(value) = map.get(key) {
                    out.insert(key.to_string(), value.clone());
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn raw_completion_summaries(
    page: usize,
    figures: &[AcademicPdfFigureCompletion],
    tables: &[AcademicPdfTableCompletion],
) -> Vec<AcademicPdfVisionRawCompletion> {
    figures
        .iter()
        .map(|item| AcademicPdfVisionRawCompletion {
            page,
            kind: "figure".to_string(),
            label: item.label.clone(),
            status: Some(item.status.clone()),
            confidence: item.confidence,
            json: item.raw.clone().unwrap_or(Value::Null),
        })
        .chain(tables.iter().map(|item| AcademicPdfVisionRawCompletion {
            page,
            kind: "table".to_string(),
            label: item.label.clone(),
            status: Some(item.status.clone()),
            confidence: item.confidence,
            json: item.raw.clone().unwrap_or(Value::Null),
        }))
        .collect()
}

fn validation(status: &str, warnings: Vec<String>) -> AcademicPdfVisionValidation {
    AcademicPdfVisionValidation {
        status: status.to_string(),
        warnings,
    }
}

fn validate_bbox(value: &[f32]) -> Option<[f32; 4]> {
    if value.len() != 4 {
        return None;
    }
    let bbox = [value[0], value[1], value[2], value[3]];
    if bbox
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0 || *value > 1.0)
    {
        return None;
    }
    if bbox[2] <= bbox[0] || bbox[3] <= bbox[1] || bbox_area(&bbox) < 0.002 {
        return None;
    }
    Some(bbox)
}

fn bbox_area(bbox: &[f32; 4]) -> f32 {
    (bbox[2] - bbox[0]) * (bbox[3] - bbox[1])
}

fn bbox_overlaps_or_touches(left: &[f32; 4], right: &[f32; 4]) -> bool {
    let margin = 0.015;
    !(left[2] + margin < right[0]
        || right[2] + margin < left[0]
        || left[3] + margin < right[1]
        || right[3] + margin < left[1])
}

fn expand_bbox(mut bbox: [f32; 4], x_pad: f32, y_pad: f32) -> [f32; 4] {
    bbox[0] = (bbox[0] - x_pad).max(0.0);
    bbox[1] = (bbox[1] - y_pad).max(0.0);
    bbox[2] = (bbox[2] + x_pad).min(1.0);
    bbox[3] = (bbox[3] + y_pad).min(1.0);
    bbox
}

fn expected_table_width(headers: &[String], rows: &[Vec<String>]) -> Option<usize> {
    if !headers.is_empty() {
        return Some(headers.len());
    }
    rows.iter().map(Vec::len).max().filter(|width| *width > 0)
}

fn flattened_exponent_like(cell: &str) -> bool {
    let compact = cell.replace(' ', "");
    compact.contains("×10") && !compact.contains('^') && !compact.contains('⁰')
}

fn synthesize_markdown(headers: &[String], rows: &[Vec<String>]) -> Option<String> {
    let width = expected_table_width(headers, rows)?;
    let headers = if headers.is_empty() {
        (1..=width)
            .map(|index| format!("col_{index}"))
            .collect::<Vec<_>>()
    } else {
        headers.to_vec()
    };
    let mut lines = Vec::new();
    lines.push(format!("| {} |", headers.join(" | ")));
    lines.push(format!("| {} |", vec!["---"; width].join(" | ")));
    for row in rows {
        let mut normalized = row.clone();
        normalized.resize(width, String::new());
        lines.push(format!("| {} |", normalized[..width].join(" | ")));
    }
    Some(lines.join("\n"))
}

fn table_repair_from_completion(completion: &AcademicPdfTableCompletion) -> AcademicPdfTableRepair {
    AcademicPdfTableRepair {
        page: completion.page,
        label: completion.label.clone(),
        status: match completion.status.as_str() {
            "reconstructed" => "ok".to_string(),
            "partial" => "uncertain".to_string(),
            "not_visible" => "missed".to_string(),
            _ => "uncertain".to_string(),
        },
        header: completion.headers.iter().take(8).cloned().collect(),
        sample_rows: completion.rows.iter().take(2).cloned().collect(),
        notes: completion.notes.clone(),
    }
}

fn figure_repair_from_completion(
    completion: &AcademicPdfFigureCompletion,
) -> AcademicPdfFigureRepair {
    AcademicPdfFigureRepair {
        page: completion.page,
        label: completion.label.clone(),
        status: match completion.status.as_str() {
            "crop_ready" => "ok".to_string(),
            "fragmented" => "uncertain".to_string(),
            "not_visible" => "missed".to_string(),
            _ => "uncertain".to_string(),
        },
        notes: completion.notes.clone(),
    }
}

fn merge_page_result(output: &mut AcademicPdfVisionArtifacts, result: VisionPageOutcome) {
    output.visual_objects.extend(result.visual_objects);
    output.table_repairs.extend(result.table_repairs);
    output.figure_repairs.extend(result.figure_repairs);
    output.figure_completions.extend(result.figure_completions);
    output.table_completions.extend(result.table_completions);
    output.raw_completions.extend(result.raw_completions);
    output.layout_warnings.extend(result.layout_warnings);
    output.calls.push(result.report);
}

async fn complete_with_backoff(
    client: &grok_search_llm::AnthropicMessagesClient,
    request: LlmRequest,
) -> Result<LlmAttemptOutcome> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempts = 0u32;
    let mut backoff_ms = 0u64;
    loop {
        attempts += 1;
        match client.complete(request.clone()).await {
            Ok(response) => {
                return Ok(LlmAttemptOutcome {
                    response,
                    attempts,
                    backoff_ms,
                });
            }
            Err(err) if attempts < MAX_ATTEMPTS && is_retryable_llm_error(&err) => {
                let delay = retry_delay_ms(attempts);
                backoff_ms = backoff_ms.saturating_add(delay);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

fn retry_delay_ms(attempt: u32) -> u64 {
    600u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(4))
}

fn is_retryable_llm_error(err: &GrokSearchError) -> bool {
    match err {
        GrokSearchError::Timeout(_) | GrokSearchError::Upstream(_) => true,
        GrokSearchError::Provider(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("http 429")
                || lower.contains("too many requests")
                || lower.contains("rate limit")
                || lower.contains("http 500")
                || lower.contains("http 502")
                || lower.contains("http 503")
                || lower.contains("http 504")
                || lower.contains("request failed")
                || lower.contains("timeout")
        }
        _ => false,
    }
}

fn selected_pages_text(pages: &[PdfVisionPage]) -> String {
    pages
        .iter()
        .map(|page| {
            format!(
                "page={}\npriority={}\nreasons={}\n{}",
                page.page_number,
                page.triage_priority,
                page.triage_reasons.join(","),
                page.markdown
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn vision_strategy_hash(
    parsed: &ParsedPdfDetails,
    pages: &[PdfVisionPage],
    page_text_sha256: &str,
    run_config: &ArtifactVisionRunConfig,
) -> String {
    let page_ids = pages
        .iter()
        .map(|page| page.page_number.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let payload = format!(
        "pdf={}\npage_text={page_text_sha256}\npages={page_ids}\nmodel={}\nprofile={}\nprompt={PROMPT_VERSION}\ndpi={}\nmax_pages={}",
        parsed.pdf_sha256,
        run_config.model,
        run_config.profile,
        run_config.render_dpi,
        run_config.max_pages,
    );
    sha256_hex(payload.as_bytes())
}

fn skipped_output(
    run_config: &ArtifactVisionRunConfig,
    pages_considered: usize,
    warning: &str,
) -> AcademicPdfVisionArtifacts {
    AcademicPdfVisionArtifacts {
        profile: run_config.profile.clone(),
        status: "skipped".to_string(),
        model: Some(run_config.model.clone()),
        pages_analyzed: 0,
        pages_considered,
        render_dpi: run_config.render_dpi,
        cache: None,
        visual_objects: Vec::new(),
        table_repairs: Vec::new(),
        figure_repairs: Vec::new(),
        figure_completions: Vec::new(),
        table_completions: Vec::new(),
        raw_completions: Vec::new(),
        layout_warnings: Vec::new(),
        calls: Vec::new(),
        warnings: vec![warning.to_string()],
    }
}

fn normalize_profile(profile: &str, config: &Config) -> Result<String> {
    match profile
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "" | PROFILE_OFF => Ok(PROFILE_OFF.to_string()),
        PROFILE_AUTO => {
            if config.llm_api_key.as_deref().is_some_and(|key| !key.trim().is_empty()) {
                Ok(PROFILE_ARTIFACT_MICRO.to_string())
            } else {
                Ok(PROFILE_OFF.to_string())
            }
        }
        PROFILE_ARTIFACT_MICRO => Ok(PROFILE_ARTIFACT_MICRO.to_string()),
        other => Err(GrokSearchError::InvalidParams(format!(
            "academic_pdf_artifacts.vision_profile must be auto, off, or artifact_micro, got {other}"
        ))),
    }
}

fn extract_json_object(raw: &str) -> String {
    let Some(start) = raw.find('{') else {
        return raw.to_string();
    };
    let Some(end) = raw.rfind('}') else {
        return raw.to_string();
    };
    raw[start..=end].to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_completion_json_and_derives_compat_repairs() {
        let raw = r#"{
          "page_number": 6,
          "figure_completions": [{
            "label": "Figure 2",
            "caption": "Figure 2 caption",
            "bbox_norm": [0.12, 0.36, 0.90, 0.54],
            "caption_bbox_norm": [0.12, 0.52, 0.90, 0.57],
            "status": "crop_ready",
            "confidence": 0.9,
            "notes": "chart"
          }],
          "table_completions": [{
            "label": "Table 2",
            "caption": "Table 2 caption",
            "bbox_norm": [0.12, 0.13, 0.90, 0.34],
            "status": "reconstructed",
            "headers": ["A","B"],
            "rows": [["x","1"],["y","2"]],
            "confidence": 0.92
          }],
          "patches": [{"kind":"replace_small_span"}],
          "layout_warnings": []
        }"#;

        let parsed = parse_page_output(6, raw, false).expect("parse");

        assert_eq!(parsed.figure_completions.len(), 1);
        assert_eq!(parsed.table_completions.len(), 1);
        assert_eq!(parsed.figure_repairs[0].status, "ok");
        assert_eq!(parsed.table_repairs[0].status, "ok");
        assert_eq!(parsed.raw_completions.len(), 2);
        assert!(parsed
            .report
            .warnings
            .contains(&"discarded_model_text_patches".to_string()));
        let figure = &parsed.figure_completions[0];
        assert!(
            figure.refined_bbox_norm[3] < 0.52,
            "caption overlap should trim figure bottom: {:?}",
            figure.refined_bbox_norm
        );
        assert_eq!(
            figure
                .validation
                .as_ref()
                .map(|value| value.status.as_str()),
            Some("ok")
        );
        assert!(parsed.table_completions[0]
            .markdown
            .as_ref()
            .expect("markdown")
            .contains("| A | B |"));
    }

    #[test]
    fn keeps_legacy_repair_shape() {
        let raw = r#"{
          "visual_objects": [{"kind":"figure","label":"Figure 1","summary":"diagram","confidence":"0.7"}],
          "table_repairs": [{"label":"Table 1","status":"usable","header":["A"],"sample_rows":[["1"]],"notes":"ok"}],
          "figure_repairs": [{"label":"Figure 1","status":"missing","notes":"caption only"}],
          "layout_warnings": ["warn"]
        }"#;

        let parsed = parse_page_output(3, raw, false).expect("parse");

        assert_eq!(parsed.visual_objects.len(), 1);
        assert_eq!(parsed.table_repairs.len(), 1);
        assert_eq!(parsed.figure_repairs.len(), 1);
        assert!(parsed.figure_completions.is_empty());
        assert!(parsed.table_completions.is_empty());
    }

    #[test]
    fn table_refine_warns_on_width_and_exponent_risks() {
        let raw = r#"{
          "table_completions": [{
            "label": "Table 1",
            "bbox_norm": [0.2, 0.1, 0.8, 0.3],
            "status": "reconstructed",
            "headers": ["k","value"],
            "rows": [["5","2.20×108","extra"]]
          }]
        }"#;

        let parsed = parse_page_output(1, raw, false).expect("parse");
        let table = &parsed.table_completions[0];
        let warnings = &table.validation.as_ref().expect("validation").warnings;

        assert!(warnings.contains(&"table_row_width_mismatch".to_string()));
        assert!(warnings
            .contains(&"table_values_may_need_superscript_or_exponent_validation".to_string()));
        assert_eq!(
            table.validation.as_ref().map(|value| value.status.as_str()),
            Some("warning")
        );
    }

    #[test]
    fn not_visible_figure_does_not_require_bbox() {
        let raw = r#"{
          "figure_completions": [{
            "label": "Figure 1",
            "status": "not_visible",
            "bbox_norm": []
          }]
        }"#;

        let parsed = parse_page_output(1, raw, false).expect("parse");
        let figure = &parsed.figure_completions[0];

        assert!(figure.refined_bbox_norm.is_empty());
        assert_eq!(
            figure
                .validation
                .as_ref()
                .map(|value| value.status.as_str()),
            Some("ok")
        );
    }
}
