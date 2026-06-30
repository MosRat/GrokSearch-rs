mod assemble;
mod cache;
mod chunking;
mod prompt;
mod schema;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use grok_search_config::Config;
use grok_search_llm::{
    AnthropicAuthScheme, AnthropicClientConfig, AnthropicMessagesClient, LlmClient, LlmMessage,
    LlmRequest, LlmRole,
};
use grok_search_pdf::ParsedPdfDetails;
use grok_search_types::{
    AcademicLlmProgressiveOptions, AcademicParseArtifact, AcademicPdfPassReport,
    AcademicProgressiveGetInput, AcademicProgressiveGetOutput, AcademicProgressivePaper,
    GrokSearchError, Result,
};
use tokio::sync::Semaphore;

use self::assemble::{assemble_paper, strip_for_output};
use self::cache::{cache_get, cache_put, strategy_hash, ProgressiveCachePlan};
use self::chunking::build_chunks;
use self::prompt::{chunk_prompt, repair_prompt};
use self::schema::{fallback_chunk_result, parse_chunk_output, ChunkResult};

const DEFAULT_MODEL: &str = "MiniMax-M3";
const DEFAULT_MAX_CHUNK_CHARS: usize = 6_500;
const DEFAULT_OVERLAP_CHARS: usize = 500;
const DEFAULT_CONCURRENCY: usize = 2;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1_600;
const DEFAULT_INPUT_PROFILE: &str = "md_light_plain_refs";
const DEFAULT_PROMPT_PROFILE: &str = "compact_v2";

#[derive(Debug, Clone)]
pub(crate) struct ProgressiveOutcome {
    pub value: Option<AcademicProgressivePaper>,
    pub artifact: Option<AcademicParseArtifact>,
    pub pass: AcademicPdfPassReport,
}

#[derive(Debug, Clone)]
pub(crate) struct ProgressiveRunConfig {
    pub model: String,
    pub max_chunk_chars: usize,
    pub overlap_chars: usize,
    pub concurrency: usize,
    pub max_output_tokens: u32,
    pub input_profile: String,
    pub prompt_profile: String,
    pub include_section_text: bool,
    pub cache_enabled: bool,
    pub cache_refresh: bool,
}

pub(crate) fn enabled(options: Option<&AcademicLlmProgressiveOptions>) -> bool {
    options.and_then(|options| options.enabled).unwrap_or(false)
}

pub(crate) async fn run(
    parsed: &ParsedPdfDetails,
    options: &AcademicLlmProgressiveOptions,
    config: &Config,
    http: &reqwest::Client,
) -> ProgressiveOutcome {
    let started = Instant::now();
    match run_inner(parsed, options, config, http).await {
        Ok(mut outcome) => {
            outcome.pass.input_length = Some(progressive_input_text(parsed).chars().count());
            outcome.pass.output_length = outcome
                .value
                .as_ref()
                .and_then(|value| serde_json::to_string(value).ok())
                .map(|value| value.chars().count());
            outcome
        }
        Err(err) => ProgressiveOutcome {
            value: None,
            artifact: None,
            pass: pass_report(
                "failed_nonfatal",
                Some(progressive_input_text(parsed).chars().count()),
                None,
                vec![format!("{err}")],
                started.elapsed().as_millis() as u64,
            ),
        },
    }
}

pub(crate) async fn get_cached(
    input: AcademicProgressiveGetInput,
    config: &Config,
) -> Result<AcademicProgressiveGetOutput> {
    let cache_key = input.cache_key.trim();
    if cache_key.is_empty() {
        return Err(GrokSearchError::InvalidParams(
            "academic_progressive_get.cache_key is required".to_string(),
        ));
    }
    let view = input.view.unwrap_or_else(|| "summary".to_string());
    if !matches!(view.as_str(), "summary" | "full" | "section") {
        return Err(GrokSearchError::InvalidParams(
            "academic_progressive_get.view must be one of summary, full, section".to_string(),
        ));
    }
    if view == "section" && input.section_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(GrokSearchError::InvalidParams(
            "academic_progressive_get.section_id is required when view=section".to_string(),
        ));
    }

    let Some(entry) =
        cache_get(config.progressive_cache_path.clone(), cache_key.to_string()).await?
    else {
        return Err(GrokSearchError::NotFound(format!(
            "progressive cache entry not found: {cache_key}"
        )));
    };
    let mut paper = serde_json::from_slice::<AcademicProgressivePaper>(&entry.bytes)
        .map_err(|err| GrokSearchError::Parse(format!("parse cached progressive paper: {err}")))?;
    if let Some(cache) = paper.cache.as_mut() {
        cache.hit = true;
        cache.stored = true;
    }
    Ok(match view.as_str() {
        "section" => {
            let section_id = input.section_id.unwrap_or_default();
            let section = paper
                .sections
                .iter()
                .find(|section| section.section_id == section_id)
                .cloned()
                .ok_or_else(|| {
                    GrokSearchError::NotFound(format!(
                        "progressive section {section_id} not found in {cache_key}"
                    ))
                })?;
            AcademicProgressiveGetOutput {
                cache_key: cache_key.to_string(),
                view,
                cache_hit: true,
                progressive_reading: None,
                section: Some(trim_section(
                    section,
                    input.include_section_text.unwrap_or(false),
                )),
                warnings: Vec::new(),
            }
        }
        "full" => AcademicProgressiveGetOutput {
            cache_key: cache_key.to_string(),
            view,
            cache_hit: true,
            progressive_reading: Some(strip_for_output(
                paper,
                input.include_section_text.unwrap_or(false),
                input.max_chars,
            )),
            section: None,
            warnings: Vec::new(),
        },
        _ => AcademicProgressiveGetOutput {
            cache_key: cache_key.to_string(),
            view,
            cache_hit: true,
            progressive_reading: Some(strip_for_output(paper, false, input.max_chars)),
            section: None,
            warnings: Vec::new(),
        },
    })
}

async fn run_inner(
    parsed: &ParsedPdfDetails,
    options: &AcademicLlmProgressiveOptions,
    config: &Config,
    http: &reqwest::Client,
) -> Result<ProgressiveOutcome> {
    let started = Instant::now();
    let run_config = ProgressiveRunConfig::from_options(options, config);
    let input_text = progressive_input_text(parsed).to_string();
    let input_text_sha256 = progressive_input_sha256(parsed, &input_text);
    let strategy_hash = strategy_hash(&run_config, &parsed.pdf_sha256, &input_text_sha256);
    let cache_key = format!("progressive:v1:{strategy_hash}");
    let cache_plan = ProgressiveCachePlan {
        path: config.progressive_cache_path.clone(),
        key: cache_key.clone(),
        ttl_seconds: Some(config.progressive_cache_ttl_seconds),
        max_entries: config.progressive_cache_max_entries,
        pdf_sha256: parsed.pdf_sha256.clone(),
        input_text_sha256: input_text_sha256.clone(),
        strategy_hash: strategy_hash.clone(),
        model: run_config.model.clone(),
        input_profile: run_config.input_profile.clone(),
        prompt_profile: run_config.prompt_profile.clone(),
    };

    let mut cache_warnings = Vec::new();
    if run_config.cache_enabled && !run_config.cache_refresh {
        match cache_get(cache_plan.path.clone(), cache_plan.key.clone()).await {
            Ok(Some(entry)) => {
                let mut paper = serde_json::from_slice::<AcademicProgressivePaper>(&entry.bytes)
                    .map_err(|err| {
                        GrokSearchError::Parse(format!("parse cached progressive paper: {err}"))
                    })?;
                if let Some(cache) = paper.cache.as_mut() {
                    cache.hit = true;
                    cache.stored = true;
                }
                let value_for_output = strip_for_output(
                    paper,
                    run_config.include_section_text,
                    config.fetch_max_chars,
                );
                return Ok(ProgressiveOutcome {
                    value: Some(value_for_output),
                    artifact: None,
                    pass: pass_report(
                        "cache_hit",
                        Some(input_text.chars().count()),
                        None,
                        Vec::new(),
                        started.elapsed().as_millis() as u64,
                    ),
                });
            }
            Ok(None) => {}
            Err(err) => cache_warnings.push(format!("cache_read_failed: {err}")),
        }
    }

    let client = build_client(config, http)?;
    let chunks = build_chunks(
        &input_text,
        run_config.max_chunk_chars,
        run_config.overlap_chars,
    );
    let semaphore = Arc::new(Semaphore::new(run_config.concurrency));
    let client = Arc::new(client);
    let mut handles = Vec::new();
    for chunk in chunks.clone() {
        let permitter = semaphore.clone();
        let client = client.clone();
        let run_config = run_config.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permitter.acquire_owned().await.map_err(|err| {
                GrokSearchError::Provider(format!("llm progressive semaphore closed: {err}"))
            })?;
            process_chunk(client.as_ref(), &run_config, chunk).await
        }));
    }

    let mut chunk_results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Ok(result)) => chunk_results.push(result),
            Ok(Err(err)) => chunk_results.push(fallback_chunk_result(format!("{err}"))),
            Err(err) => chunk_results.push(fallback_chunk_result(format!("join error: {err}"))),
        }
    }
    let mut paper = assemble_paper(
        &input_text,
        parsed.progressive_source.as_ref(),
        &chunks,
        &chunk_results,
        &run_config,
        started.elapsed().as_millis() as u64,
    );
    paper.llm_report.warnings.extend(cache_warnings.clone());
    paper.cache = Some(grok_search_types::AcademicProgressiveCacheInfo {
        key: cache_key,
        hit: false,
        stored: false,
        strategy_hash,
        path: Some(config.progressive_cache_path.display().to_string()),
        warnings: cache_warnings,
    });

    if run_config.cache_enabled {
        match serde_json::to_vec(&paper)
            .map_err(|err| GrokSearchError::Parse(format!("serialize progressive paper: {err}")))
        {
            Ok(bytes) => match cache_put(cache_plan, bytes).await {
                Ok(()) => {
                    if let Some(cache) = paper.cache.as_mut() {
                        cache.stored = true;
                    }
                }
                Err(err) => {
                    if let Some(cache) = paper.cache.as_mut() {
                        cache.warnings.push(format!("cache_write_failed: {err}"));
                    }
                    paper
                        .llm_report
                        .warnings
                        .push(format!("cache_write_failed: {err}"));
                }
            },
            Err(err) => {
                if let Some(cache) = paper.cache.as_mut() {
                    cache
                        .warnings
                        .push(format!("cache_serialize_failed: {err}"));
                }
            }
        }
    }

    let mut artifact = None;
    if let Some(path) = options.save_json_path.as_deref() {
        artifact = Some(write_progressive_json(path, &paper)?);
    }

    let value_for_output = strip_for_output(
        paper,
        run_config.include_section_text,
        config.fetch_max_chars,
    );
    Ok(ProgressiveOutcome {
        value: Some(value_for_output),
        artifact,
        pass: pass_report(
            "ok",
            Some(input_text.chars().count()),
            None,
            Vec::new(),
            started.elapsed().as_millis() as u64,
        ),
    })
}

impl ProgressiveRunConfig {
    fn from_options(options: &AcademicLlmProgressiveOptions, config: &Config) -> Self {
        Self {
            model: options
                .model
                .clone()
                .or_else(|| nonempty(config.progressive_default_model.clone()))
                .or_else(|| nonempty(config.llm_model.clone()))
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            max_chunk_chars: options
                .max_chunk_chars
                .unwrap_or(DEFAULT_MAX_CHUNK_CHARS)
                .clamp(1_000, 20_000),
            overlap_chars: options
                .overlap_chars
                .unwrap_or(DEFAULT_OVERLAP_CHARS)
                .min(5_000),
            concurrency: options
                .concurrency
                .unwrap_or(DEFAULT_CONCURRENCY)
                .clamp(1, 4),
            max_output_tokens: options
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
                .clamp(256, 4_000),
            input_profile: options
                .input_profile
                .clone()
                .unwrap_or_else(|| DEFAULT_INPUT_PROFILE.to_string()),
            prompt_profile: options
                .prompt_profile
                .clone()
                .unwrap_or_else(|| DEFAULT_PROMPT_PROFILE.to_string()),
            include_section_text: options.include_section_text.unwrap_or(false),
            cache_enabled: config.progressive_cache_enabled
                && options.cache_enabled.unwrap_or(true),
            cache_refresh: options.cache_refresh.unwrap_or(false),
        }
    }
}

async fn process_chunk(
    client: &AnthropicMessagesClient,
    config: &ProgressiveRunConfig,
    chunk: chunking::Chunk,
) -> Result<ChunkResult> {
    let started = Instant::now();
    let prompt = chunk_prompt(&chunk, &config.prompt_profile);
    let mut request = LlmRequest::new(&config.model, vec![LlmMessage::text(LlmRole::User, prompt)]);
    request.system = Some(
        "Return valid JSON only. Do not invent facts beyond cited anchors. Prefer omission over guessing."
            .to_string(),
    );
    request.max_tokens = Some(config.max_output_tokens);
    request.temperature = Some(0.0);
    let response = client.complete(request).await?;
    let output = response
        .content
        .iter()
        .find_map(|block| block.as_text())
        .unwrap_or_default()
        .to_string();
    match parse_chunk_output(&chunk, &output, started.elapsed().as_millis() as u64, false) {
        Ok(result) => Ok(result),
        Err(first_err) => {
            let mut repair_request = LlmRequest::new(
                &config.model,
                vec![LlmMessage::text(
                    LlmRole::User,
                    repair_prompt(&output, &first_err.to_string()),
                )],
            );
            repair_request.system = Some("Return repaired valid JSON only.".to_string());
            repair_request.max_tokens = Some(config.max_output_tokens.min(1_000));
            repair_request.temperature = Some(0.0);
            let repaired = client.complete(repair_request).await?;
            let repaired_output = repaired
                .content
                .iter()
                .find_map(|block| block.as_text())
                .unwrap_or_default()
                .to_string();
            parse_chunk_output(
                &chunk,
                &repaired_output,
                started.elapsed().as_millis() as u64,
                true,
            )
            .or_else(|err| {
                Ok(fallback_chunk_result(format!(
                    "invalid_llm_json: {first_err}; repair_failed: {err}"
                )))
            })
        }
    }
}

fn build_client(config: &Config, http: &reqwest::Client) -> Result<AnthropicMessagesClient> {
    let env_file = read_env_llm();
    let provider = config.llm_provider.trim().to_ascii_lowercase();
    if !matches!(provider.as_str(), "" | "anthropic" | "minimax") {
        return Err(GrokSearchError::InvalidParams(format!(
            "llm_provider must be anthropic or minimax for academic_pdf_structure, got {provider}"
        )));
    }
    let api_key = config
        .llm_api_key
        .clone()
        .or_else(|| env_lookup(&env_file, "ANTHROPIC_API_KEY"))
        .or_else(|| env_lookup(&env_file, "MINIMAX_API_KEY"))
        .ok_or(GrokSearchError::MissingConfig(
            "llm_api_key or GROK_SEARCH_LLM_API_KEY",
        ))?;
    let mut llm_config = AnthropicClientConfig::minimax(api_key);
    llm_config.base_url = nonempty(config.llm_base_url.clone())
        .or_else(|| env_lookup(&env_file, "ANTHROPIC_BASE_URL"))
        .unwrap_or_else(|| grok_search_llm::DEFAULT_MINIMAX_ANTHROPIC_BASE_URL.to_string());
    llm_config.auth_scheme = llm_auth_scheme(&config.llm_auth_scheme)?;
    llm_config.max_response_bytes = config.max_response_bytes;
    Ok(AnthropicMessagesClient::new(http.clone(), llm_config))
}

fn progressive_input_text(parsed: &ParsedPdfDetails) -> &str {
    parsed
        .progressive_source
        .as_ref()
        .map(|source| source.text.as_str())
        .unwrap_or(parsed.content.as_str())
}

fn progressive_input_sha256(parsed: &ParsedPdfDetails, input_text: &str) -> String {
    parsed
        .progressive_source
        .as_ref()
        .map(|source| source.text_sha256.clone())
        .unwrap_or_else(|| sha256_hex(input_text.as_bytes()))
}

fn env_lookup(file: &HashMap<String, String>, key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .or_else(|| file.get(key).cloned())
        .filter(|value| !value.trim().is_empty())
}

fn nonempty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn llm_auth_scheme(raw: &str) -> Result<AnthropicAuthScheme> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "bearer" => Ok(AnthropicAuthScheme::Bearer),
        "x-api-key" | "x_api_key" | "api-key" | "api_key" => Ok(AnthropicAuthScheme::XApiKey),
        "both" => Ok(AnthropicAuthScheme::Both),
        other => Err(GrokSearchError::InvalidParams(format!(
            "llm_auth_scheme must be bearer, x-api-key, or both, got {other}"
        ))),
    }
}

fn read_env_llm() -> HashMap<String, String> {
    let mut out = HashMap::new();
    for path in [PathBuf::from(".env.llm"), PathBuf::from("../.env.llm")] {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in text.lines() {
            if let Some((key, value)) = parse_env_line(line) {
                out.insert(key, value);
            }
        }
    }
    out
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let line = line.trim().trim_start_matches('\u{feff}');
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line
        .strip_prefix("export")
        .map(str::trim_start)
        .unwrap_or(line)
        .trim();
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), unquote(value.trim()).to_string()))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn write_progressive_json(
    path: &str,
    value: &AcademicProgressivePaper,
) -> Result<AcademicParseArtifact> {
    let path = PathBuf::from(path);
    if path.exists() {
        return Err(GrokSearchError::InvalidParams(format!(
            "progressive JSON output path already exists: {}",
            path.display()
        )));
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            GrokSearchError::Io(format!(
                "create progressive JSON output directory {}: {err}",
                parent.display()
            ))
        })?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| GrokSearchError::Parse(format!("serialize progressive JSON: {err}")))?;
    std::fs::write(&path, text.as_bytes()).map_err(|err| {
        GrokSearchError::Io(format!("write progressive JSON {}: {err}", path.display()))
    })?;
    Ok(AcademicParseArtifact {
        kind: "progressive_reading".to_string(),
        status: "written".to_string(),
        path: Some(path.display().to_string()),
        count: None,
        bytes: Some(text.len() as u64),
        reason: None,
    })
}

fn trim_section(
    mut section: grok_search_types::AcademicProgressiveSection,
    include_text: bool,
) -> grok_search_types::AcademicProgressiveSection {
    if !include_text {
        section.clean_text = None;
    }
    section
}

fn pass_report(
    status: &str,
    input_length: Option<usize>,
    output_length: Option<usize>,
    warnings: Vec<String>,
    elapsed_ms: u64,
) -> AcademicPdfPassReport {
    let mut warnings = warnings;
    warnings.push(format!("elapsed_ms={elapsed_ms}"));
    AcademicPdfPassReport {
        name: "llm_progressive_reading".to_string(),
        status: status.to_string(),
        input_length,
        output_length,
        warnings,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
