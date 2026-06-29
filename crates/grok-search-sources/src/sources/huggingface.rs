use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::Client;
use url::Url;

use crate::sources::{get_json, get_text};
use grok_search_types::{
    GrokSearchError, RepoKind, RepoLinks, RepoMetadataOutput, RepoProvider, Result,
};

use super::github::limit_repo_text;

const UA: &str = "grok-search-rs/0.1 (https://github.com/MosRat/GrokSearch-rs)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuggingFaceRepoType {
    Model,
    Dataset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuggingFaceRepoLocator {
    pub repo_id: String,
    pub repo_type: HuggingFaceRepoType,
}

pub fn parse_repo_url(url: &Url) -> Result<HuggingFaceRepoLocator> {
    if url.host_str() != Some("huggingface.co") {
        return Err(GrokSearchError::InvalidParams(
            "huggingface repo URL must use huggingface.co".into(),
        ));
    }
    let segs: Vec<String> = url
        .path_segments()
        .map(|it| it.filter(|s| !s.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    if segs.is_empty() {
        return Err(GrokSearchError::InvalidParams(
            "huggingface repo URL must include a repo id".into(),
        ));
    }
    if segs[0] == "spaces" {
        return Err(GrokSearchError::InvalidParams(
            "repo_metadata supports Hugging Face models and datasets; spaces are not supported"
                .into(),
        ));
    }
    if segs[0] == "datasets" {
        let repo_id = repo_id_from_segments(&segs[1..])?;
        return Ok(HuggingFaceRepoLocator {
            repo_id,
            repo_type: HuggingFaceRepoType::Dataset,
        });
    }
    if matches!(segs[0].as_str(), "api" | "docs" | "models") {
        return Err(GrokSearchError::InvalidParams(
            "huggingface repo URL must be a public model or dataset page".into(),
        ));
    }
    Ok(HuggingFaceRepoLocator {
        repo_id: repo_id_from_segments(&segs)?,
        repo_type: HuggingFaceRepoType::Model,
    })
}

pub fn repo_locator(repo_id: &str, repo_type: Option<&str>) -> Result<HuggingFaceRepoLocator> {
    let repo_id = repo_id.trim().trim_matches('/').to_string();
    if repo_id.is_empty() {
        return Err(GrokSearchError::InvalidParams(
            "huggingface repo_id is required".into(),
        ));
    }
    if repo_id.starts_with("spaces/") {
        return Err(GrokSearchError::InvalidParams(
            "repo_metadata supports Hugging Face models and datasets; spaces are not supported"
                .into(),
        ));
    }
    if repo_id.starts_with("datasets/") {
        return Ok(HuggingFaceRepoLocator {
            repo_id: repo_id.trim_start_matches("datasets/").to_string(),
            repo_type: HuggingFaceRepoType::Dataset,
        });
    }
    let repo_type =
        match repo_type.unwrap_or("model") {
            "model" | "models" => HuggingFaceRepoType::Model,
            "dataset" | "datasets" => HuggingFaceRepoType::Dataset,
            "space" | "spaces" => return Err(GrokSearchError::InvalidParams(
                "repo_metadata supports Hugging Face models and datasets; spaces are not supported"
                    .into(),
            )),
            other => {
                return Err(GrokSearchError::InvalidParams(format!(
                    "huggingface repo_type must be model or dataset, got {other}"
                )))
            }
        };
    Ok(HuggingFaceRepoLocator { repo_id, repo_type })
}

pub async fn fetch_repo_metadata(
    client: &Client,
    locator: &HuggingFaceRepoLocator,
    token: Option<&str>,
    include_card: bool,
    max_text_chars: Option<usize>,
) -> Result<RepoMetadataOutput> {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(reqwest::header::HeaderName, &str)> = vec![(USER_AGENT, UA)];
    if let Some(ref a) = auth {
        headers.push((AUTHORIZATION, a.as_str()));
    }

    let api = api_url(locator);
    let json = get_json(client, &api, &headers, "huggingface_repo").await?;
    let mut output = metadata_from_json(locator, &api, &json);
    if include_card {
        let card_url = card_url(locator);
        match get_text(client, &card_url, &headers, "huggingface_card").await {
            Ok(text) if !text.trim().is_empty() => {
                output.card = Some(limit_repo_text(text, max_text_chars));
                output.links.card = Some(card_url);
            }
            Ok(_) => output.warnings.push("huggingface card empty".to_string()),
            Err(err) => output
                .warnings
                .push(format!("huggingface card skipped: {err}")),
        }
    }
    Ok(output)
}

fn repo_id_from_segments(segs: &[String]) -> Result<String> {
    if segs.is_empty() || segs.len() > 2 {
        return Err(GrokSearchError::InvalidParams(
            "huggingface repo id must be {name} or {namespace}/{name}".into(),
        ));
    }
    Ok(segs.join("/"))
}

fn api_url(locator: &HuggingFaceRepoLocator) -> String {
    match locator.repo_type {
        HuggingFaceRepoType::Model => {
            format!("https://huggingface.co/api/models/{}", locator.repo_id)
        }
        HuggingFaceRepoType::Dataset => {
            format!("https://huggingface.co/api/datasets/{}", locator.repo_id)
        }
    }
}

fn html_url(locator: &HuggingFaceRepoLocator) -> String {
    match locator.repo_type {
        HuggingFaceRepoType::Model => format!("https://huggingface.co/{}", locator.repo_id),
        HuggingFaceRepoType::Dataset => {
            format!("https://huggingface.co/datasets/{}", locator.repo_id)
        }
    }
}

fn card_url(locator: &HuggingFaceRepoLocator) -> String {
    match locator.repo_type {
        HuggingFaceRepoType::Model => {
            format!(
                "https://huggingface.co/{}/raw/main/README.md",
                locator.repo_id
            )
        }
        HuggingFaceRepoType::Dataset => {
            format!(
                "https://huggingface.co/datasets/{}/raw/main/README.md",
                locator.repo_id
            )
        }
    }
}

fn metadata_from_json(
    locator: &HuggingFaceRepoLocator,
    api: &str,
    json: &serde_json::Value,
) -> RepoMetadataOutput {
    let id = json
        .get("id")
        .or_else(|| json.get("modelId"))
        .and_then(|v| v.as_str())
        .unwrap_or(&locator.repo_id)
        .to_string();
    let (owner, name) = split_owner_name(&id);
    let card_data = json.get("cardData").unwrap_or(&serde_json::Value::Null);
    let license = card_data
        .get("license")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut tags = json
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    tags.sort();
    tags.dedup();

    RepoMetadataOutput {
        provider: RepoProvider::Huggingface,
        kind: match locator.repo_type {
            HuggingFaceRepoType::Model => RepoKind::HuggingfaceModel,
            HuggingFaceRepoType::Dataset => RepoKind::HuggingfaceDataset,
        },
        id,
        name,
        owner,
        description: card_data
            .get("description")
            .or_else(|| json.get("description"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        license,
        tags,
        stars: None,
        forks: None,
        downloads: json.get("downloads").and_then(|v| v.as_u64()),
        likes: json.get("likes").and_then(|v| v.as_u64()),
        created_at: json
            .get("createdAt")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        updated_at: json
            .get("lastModified")
            .or_else(|| json.get("updatedAt"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        default_branch: None,
        links: RepoLinks {
            html: html_url(locator),
            api: api.to_string(),
            readme: None,
            card: None,
        },
        readme: None,
        card: None,
        warnings: Vec::new(),
    }
}

fn split_owner_name(id: &str) -> (Option<String>, String) {
    match id.rsplit_once('/') {
        Some((owner, name)) => (Some(owner.to_string()), name.to_string()),
        None => (None, id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_url() {
        let url = Url::parse("https://huggingface.co/openai/whisper-large-v3").unwrap();
        let locator = parse_repo_url(&url).unwrap();
        assert_eq!(locator.repo_id, "openai/whisper-large-v3");
        assert_eq!(locator.repo_type, HuggingFaceRepoType::Model);
    }

    #[test]
    fn parses_dataset_url() {
        let url = Url::parse("https://huggingface.co/datasets/squad").unwrap();
        let locator = parse_repo_url(&url).unwrap();
        assert_eq!(locator.repo_id, "squad");
        assert_eq!(locator.repo_type, HuggingFaceRepoType::Dataset);
    }

    #[test]
    fn rejects_space_url() {
        let url = Url::parse("https://huggingface.co/spaces/org/demo").unwrap();
        let err = parse_repo_url(&url).unwrap_err();
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn repo_locator_defaults_to_model() {
        let locator = repo_locator("bert-base-uncased", None).unwrap();
        assert_eq!(locator.repo_id, "bert-base-uncased");
        assert_eq!(locator.repo_type, HuggingFaceRepoType::Model);
    }

    #[test]
    fn metadata_maps_model_json() {
        let locator = repo_locator("openai/whisper-large-v3", Some("model")).unwrap();
        let json = serde_json::json!({
            "id": "openai/whisper-large-v3",
            "likes": 7,
            "downloads": 11,
            "tags": ["automatic-speech-recognition", "automatic-speech-recognition"],
            "cardData": { "license": "mit", "description": "speech model" },
            "createdAt": "2024-01-01T00:00:00.000Z",
            "lastModified": "2024-02-01T00:00:00.000Z"
        });
        let output = metadata_from_json(&locator, "https://api.example", &json);
        assert_eq!(output.kind, RepoKind::HuggingfaceModel);
        assert_eq!(output.owner.as_deref(), Some("openai"));
        assert_eq!(output.name, "whisper-large-v3");
        assert_eq!(output.tags, vec!["automatic-speech-recognition"]);
        assert_eq!(output.license.as_deref(), Some("mit"));
    }
}
