use std::time::Instant;

use grok_search_net::url_policy::validate_public_http_url;
use grok_search_sources::sources::{github, huggingface};
use grok_search_types::{
    GrokSearchError, RepoMetadataInput, RepoMetadataOutput, RepoProvider, Result,
};
use serde_json::json;
use url::Url;

use crate::fetch::summarize_url;
use crate::service::SearchService;

impl SearchService {
    pub async fn repo_metadata(&self, input: RepoMetadataInput) -> Result<RepoMetadataOutput> {
        let op_start = Instant::now();
        let request_id = self.logger.request_id();
        self.logger.event(
            &request_id,
            "debug",
            "repo_metadata.start",
            Some("repo_metadata"),
            None,
            json!({
                "url": input.url.as_deref().map(summarize_url),
                "provider": input.provider,
                "repo_id_present": input.repo_id.as_ref().is_some_and(|v| !v.trim().is_empty()),
                "owner_present": input.owner.as_ref().is_some_and(|v| !v.trim().is_empty()),
                "name_present": input.name.as_ref().is_some_and(|v| !v.trim().is_empty()),
                "repo_type": input.repo_type,
                "include_readme": input.include_readme,
                "include_card": input.include_card,
                "max_text_chars": input.max_text_chars,
            }),
        );
        let result = self.repo_metadata_inner(input).await;
        match &result {
            Ok(output) => self.logger.event(
                &request_id,
                "debug",
                "repo_metadata.success",
                Some("repo_metadata"),
                Some(op_start.elapsed()),
                json!({
                    "provider": output.provider,
                    "kind": output.kind,
                    "id": output.id,
                    "warnings": output.warnings.len(),
                }),
            ),
            Err(err) => self.logger.error(
                &request_id,
                "repo_metadata.error",
                Some("repo_metadata"),
                Some(op_start.elapsed()),
                err,
                json!({}),
            ),
        }
        result
    }

    async fn repo_metadata_inner(&self, input: RepoMetadataInput) -> Result<RepoMetadataOutput> {
        let max_text_chars = input.max_text_chars.or(self.config.fetch_max_chars);
        match resolve_repo_locator(&input)? {
            RepoLocator::Github(locator) => {
                github::fetch_repo_metadata(
                    &self.http_client,
                    &locator,
                    self.config.github_token.as_deref(),
                    input.include_readme.unwrap_or(false),
                    max_text_chars,
                )
                .await
            }
            RepoLocator::HuggingFace(locator) => {
                huggingface::fetch_repo_metadata(
                    &self.http_client,
                    &locator,
                    huggingface_token().as_deref(),
                    input.include_card.unwrap_or(false),
                    max_text_chars,
                )
                .await
            }
        }
    }
}

#[derive(Debug)]
enum RepoLocator {
    Github(github::GithubRepoLocator),
    HuggingFace(huggingface::HuggingFaceRepoLocator),
}

fn resolve_repo_locator(input: &RepoMetadataInput) -> Result<RepoLocator> {
    if let Some(raw_url) = input.url.as_deref().filter(|v| !v.trim().is_empty()) {
        validate_public_http_url(raw_url)?;
        let url = Url::parse(raw_url)
            .map_err(|err| GrokSearchError::InvalidParams(format!("repo_metadata.url: {err}")))?;
        return match url.host_str() {
            Some("github.com") => Ok(RepoLocator::Github(github::parse_repo_url(&url)?)),
            Some("huggingface.co") => {
                Ok(RepoLocator::HuggingFace(huggingface::parse_repo_url(&url)?))
            }
            Some(host) => Err(GrokSearchError::InvalidParams(format!(
                "repo_metadata supports github.com and huggingface.co, got {host}"
            ))),
            None => Err(GrokSearchError::InvalidParams(
                "repo_metadata.url must include a host".into(),
            )),
        };
    }

    let provider = input.provider.as_ref().ok_or_else(|| {
        GrokSearchError::InvalidParams(
            "repo_metadata requires url or provider with repo_id/owner/name".into(),
        )
    })?;
    match provider {
        RepoProvider::Github => {
            let (owner, name) = owner_name(input)?;
            Ok(RepoLocator::Github(github::repo_locator(owner, name)?))
        }
        RepoProvider::Huggingface => {
            let repo_id = input
                .repo_id
                .clone()
                .or_else(|| {
                    input.owner.as_ref().and_then(|owner| {
                        input
                            .name
                            .as_ref()
                            .map(|name| format!("{}/{}", owner.trim(), name.trim()))
                    })
                })
                .ok_or_else(|| {
                    GrokSearchError::InvalidParams(
                        "huggingface repo_metadata requires repo_id or owner/name".into(),
                    )
                })?;
            Ok(RepoLocator::HuggingFace(huggingface::repo_locator(
                &repo_id,
                input.repo_type.as_deref(),
            )?))
        }
    }
}

fn owner_name(input: &RepoMetadataInput) -> Result<(&str, &str)> {
    let owner = input.owner.as_deref().ok_or_else(|| {
        GrokSearchError::InvalidParams("github repo_metadata requires owner and name".into())
    })?;
    let name = input.name.as_deref().ok_or_else(|| {
        GrokSearchError::InvalidParams("github repo_metadata requires owner and name".into())
    })?;
    Ok((owner, name))
}

fn huggingface_token() -> Option<String> {
    std::env::var("HF_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("HUGGINGFACE_TOKEN")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_search_types::RepoKind;

    #[test]
    fn resolves_github_url() {
        let input = RepoMetadataInput {
            url: Some("https://github.com/owner/repo".to_string()),
            ..Default::default()
        };
        match resolve_repo_locator(&input).unwrap() {
            RepoLocator::Github(locator) => {
                assert_eq!(locator.owner, "owner");
                assert_eq!(locator.repo, "repo");
            }
            RepoLocator::HuggingFace(_) => panic!("expected github locator"),
        }
    }

    #[test]
    fn resolves_huggingface_dataset_url() {
        let input = RepoMetadataInput {
            url: Some("https://huggingface.co/datasets/squad".to_string()),
            ..Default::default()
        };
        match resolve_repo_locator(&input).unwrap() {
            RepoLocator::HuggingFace(locator) => {
                assert_eq!(locator.repo_id, "squad");
                assert_eq!(locator.repo_type, huggingface::HuggingFaceRepoType::Dataset);
            }
            RepoLocator::Github(_) => panic!("expected huggingface locator"),
        }
    }

    #[test]
    fn rejects_huggingface_space_url() {
        let input = RepoMetadataInput {
            url: Some("https://huggingface.co/spaces/org/demo".to_string()),
            ..Default::default()
        };
        let err = resolve_repo_locator(&input).unwrap_err();
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn resolves_huggingface_repo_id_default_model() {
        let input = RepoMetadataInput {
            provider: Some(RepoProvider::Huggingface),
            repo_id: Some("bert-base-uncased".to_string()),
            ..Default::default()
        };
        match resolve_repo_locator(&input).unwrap() {
            RepoLocator::HuggingFace(locator) => {
                assert_eq!(locator.repo_id, "bert-base-uncased");
                assert_eq!(locator.repo_type, huggingface::HuggingFaceRepoType::Model);
            }
            RepoLocator::Github(_) => panic!("expected huggingface locator"),
        }
    }

    #[test]
    fn rejects_unknown_url_host() {
        let input = RepoMetadataInput {
            url: Some("https://example.com/owner/repo".to_string()),
            ..Default::default()
        };
        let err = resolve_repo_locator(&input).unwrap_err();
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[test]
    fn repo_kind_is_serializable_for_logs() {
        let value = serde_json::to_value(RepoKind::HuggingfaceDataset).unwrap();
        assert_eq!(value, "huggingface_dataset");
    }
}
