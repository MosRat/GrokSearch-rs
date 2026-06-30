use grok_search_llm::{
    AnthropicClientConfig, AnthropicMessagesClient, LlmClient, LlmMessage, LlmRequest, LlmRole,
    DEFAULT_MINIMAX_ANTHROPIC_BASE_URL,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[tokio::test]
#[ignore = "requires .env.llm or ANTHROPIC_API_KEY and live MiniMax network access"]
async fn minimax_anthropic_compatible_text_completion_for_pdf_pipeline() {
    let env = load_env_llm();
    let api_key = env_value(&env, "ANTHROPIC_API_KEY")
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .expect("set ANTHROPIC_API_KEY in .env.llm or process env");
    let base_url = env_value(&env, "ANTHROPIC_BASE_URL")
        .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
        .unwrap_or_else(|| DEFAULT_MINIMAX_ANTHROPIC_BASE_URL.to_string());
    let model = env_value(&env, "ANTHROPIC_MODEL")
        .or_else(|| std::env::var("ANTHROPIC_MODEL").ok())
        .unwrap_or_else(|| "MiniMax-M3".to_string());

    let mut config = AnthropicClientConfig::minimax(api_key);
    config.base_url = base_url;
    config.max_response_bytes = 1024 * 1024;

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .no_proxy()
        .build()
        .expect("client");
    let client = AnthropicMessagesClient::new(http, config);
    let mut request = LlmRequest::new(
        model,
        vec![LlmMessage::text(
            LlmRole::User,
            "You are checking a PDF parsing pipeline. Return exactly this JSON: {\"ok\":true,\"task\":\"pdf\"}",
        )],
    );
    request.system = Some("Answer with compact JSON only. Do not use markdown.".to_string());
    request.max_tokens = Some(64);
    request.temperature = Some(0.0);

    let response = client.complete(request).await.expect("MiniMax response");
    let text = response
        .content
        .iter()
        .find_map(|block| block.as_text())
        .unwrap_or_default();
    assert!(
        text.contains("\"ok\"") || text.contains("ok"),
        "unexpected response text: {text}"
    );
}

fn load_env_llm() -> HashMap<String, String> {
    find_env_llm_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|content| {
            content
                .lines()
                .filter_map(parse_env_line)
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default()
}

fn find_env_llm_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.extend(ancestor_env_paths(&cwd));
    }
    candidates.extend(ancestor_env_paths(Path::new(env!("CARGO_MANIFEST_DIR"))));
    candidates.into_iter().find(|path| path.is_file())
}

fn ancestor_env_paths(start: &Path) -> Vec<PathBuf> {
    start.ancestors().map(|dir| dir.join(".env.llm")).collect()
}

fn env_value(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key).filter(|value| !value.is_empty()).cloned()
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
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
                .strip_prefix('“')
                .and_then(|value| value.strip_suffix('”'))
        })
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_line_accepts_export_and_quotes() {
        assert_eq!(
            parse_env_line("export ANTHROPIC_BASE_URL=\"https://api.minimaxi.com/anthropic\""),
            Some((
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.minimaxi.com/anthropic".to_string()
            ))
        );
    }

    #[test]
    fn parse_env_line_accepts_export_with_spaces() {
        assert_eq!(
            parse_env_line("export ANTHROPIC_API_KEY = 'secret'"),
            Some(("ANTHROPIC_API_KEY".to_string(), "secret".to_string()))
        );
    }
}
