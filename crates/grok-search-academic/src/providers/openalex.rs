use std::time::Duration;

use async_trait::async_trait;
use grok_search_net::http::DEFAULT_MAX_RESPONSE_BYTES;
use grok_search_net::key_pool::{is_key_scoped_status, KeyPool};
use grok_search_parse::openalex_abstract;
use grok_search_provider_core::{
    AcademicIdentifier as Identifier, AcademicProvider, FullTextLocation,
};
use grok_search_types::{
    AcademicCitationSummary, AcademicPaper, AcademicSearchInput, GrokSearchError, Result,
};
use serde_json::Value;
use url::Url;

use super::http::{get_json_with_status, retry_delay, GetJsonFailure};
use super::sort_is;
use crate::service::{as_u32, source};

const OPENALEX_TRANSIENT_MAX_RETRIES: usize = 2;

#[derive(Clone)]
pub(crate) struct OpenAlexProvider {
    client: reqwest::Client,
    email: Option<String>,
    keys: Option<KeyPool>,
    max_response_bytes: usize,
}

impl OpenAlexProvider {
    #[allow(dead_code)]
    pub(crate) fn new(
        client: reqwest::Client,
        email: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        Self::new_with_limit(client, email, api_key, DEFAULT_MAX_RESPONSE_BYTES)
    }

    pub(crate) fn new_with_limit(
        client: reqwest::Client,
        email: Option<String>,
        api_key: Option<String>,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            client,
            email,
            keys: api_key.map(|key| KeyPool::parse(&key)),
            max_response_bytes,
        }
    }

    pub(super) fn add_mailto(&self, url: &mut Url) {
        if let Some(email) = &self.email {
            url.query_pairs_mut().append_pair("mailto", email);
        }
    }

    pub(super) fn add_key(&self, url: &mut Url, index: usize) {
        if let Some(keys) = &self.keys {
            url.query_pairs_mut()
                .append_pair("api_key", keys.key(index));
        }
    }

    pub(super) async fn get_json(&self, base_url: &Url, label: &str) -> Result<Value> {
        let Some(keys) = &self.keys else {
            return self.get_json_with_transient_retries(base_url, label).await;
        };
        let start = keys.start();
        let mut last_key_error = None;
        for offset in 0..keys.len() {
            let mut url = base_url.clone();
            self.add_key(&mut url, start + offset);
            match self.get_json_with_key_retries(&url, label).await {
                Ok(value) => return Ok(value),
                Err(failure) => {
                    if failure.status.is_some_and(is_key_scoped_status) && offset + 1 < keys.len() {
                        last_key_error = Some(failure.error);
                        continue;
                    }
                    return Err(failure.error);
                }
            }
        }
        Err(last_key_error.unwrap_or_else(|| {
            GrokSearchError::Config(format!("{label} request failed with no configured key"))
        }))
    }

    async fn get_json_with_transient_retries(&self, url: &Url, label: &str) -> Result<Value> {
        for attempt in 0..=OPENALEX_TRANSIENT_MAX_RETRIES {
            match get_json_with_status(&self.client, url.as_str(), label, self.max_response_bytes)
                .await
            {
                Ok(value) => return Ok(value),
                Err(failure)
                    if failure.status.is_some_and(is_transient_gateway_status)
                        && attempt < OPENALEX_TRANSIENT_MAX_RETRIES =>
                {
                    tokio::time::sleep(retry_delay(
                        failure.retry_after,
                        openalex_transient_delay(attempt),
                    ))
                    .await;
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(GrokSearchError::Upstream(format!(
            "{label} transient gateway failure after retries"
        )))
    }

    async fn get_json_with_key_retries(
        &self,
        url: &Url,
        label: &str,
    ) -> std::result::Result<Value, GetJsonFailure> {
        for attempt in 0..=OPENALEX_TRANSIENT_MAX_RETRIES {
            match get_json_with_status(&self.client, url.as_str(), label, self.max_response_bytes)
                .await
            {
                Ok(value) => return Ok(value),
                Err(failure)
                    if failure.status.is_some_and(is_transient_gateway_status)
                        && attempt < OPENALEX_TRANSIENT_MAX_RETRIES =>
                {
                    tokio::time::sleep(retry_delay(
                        failure.retry_after,
                        openalex_transient_delay(attempt),
                    ))
                    .await;
                }
                Err(failure) => return Err(failure),
            }
        }
        unreachable!("OpenAlex transient retry loop always returns from match arms")
    }
}

#[async_trait]
impl AcademicProvider for OpenAlexProvider {
    fn name(&self) -> &'static str {
        "openalex"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://api.openalex.org/works").unwrap();
        url.query_pairs_mut()
            .append_pair("search.title_and_abstract", &input.query)
            .append_pair("per-page", &limit.to_string());
        if let Some(filter) = openalex_filter(input) {
            url.query_pairs_mut().append_pair("filter", &filter);
        }
        if let Some(sort) = openalex_sort(input) {
            url.query_pairs_mut().append_pair("sort", sort);
        }
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(value
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_openalex_work)
            .map(without_openalex_reference_sources)
            .collect())
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let id = match identifier {
            Identifier::OpenAlex(id) => id.clone(),
            Identifier::Doi(doi) => format!("doi:{doi}"),
            _ => return Ok(None),
        };
        let mut url = Url::parse(&format!("https://api.openalex.org/works/{id}")).unwrap();
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(Some(parse_openalex_work(&value)))
    }

    async fn citations(
        &self,
        identifier: &Identifier,
        limit: usize,
    ) -> Result<Option<AcademicCitationSummary>> {
        let Some(work) = self.get(identifier).await? else {
            return Ok(None);
        };
        let mut summary = AcademicCitationSummary::default();
        if let Some(openalex_id) = &work.openalex_id {
            let mut cited_by = Url::parse("https://api.openalex.org/works").unwrap();
            cited_by
                .query_pairs_mut()
                .append_pair("filter", &format!("cites:{openalex_id}"))
                .append_pair("per-page", &limit.to_string());
            self.add_mailto(&mut cited_by);
            if let Ok(value) = self.get_json(&cited_by, "openalex citations").await {
                summary.citations = value
                    .get("results")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(parse_openalex_work)
                    .collect();
            }
            if let Ok(Some(detail)) = self.get(&Identifier::OpenAlex(openalex_id.clone())).await {
                for referenced in detail
                    .sources
                    .iter()
                    .filter(|s| s.provider.as_ref() == "openalex_reference")
                    .take(limit)
                {
                    summary.references.push(AcademicPaper {
                        id: referenced.url.clone(),
                        title: referenced
                            .title
                            .clone()
                            .unwrap_or_else(|| referenced.url.clone()),
                        openalex_id: Some(referenced.url.clone()),
                        url: Some(referenced.url.clone()),
                        sources: vec![referenced.clone()],
                        ..Default::default()
                    });
                }
            }
        }
        Ok(Some(summary))
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let Some(id) = &paper.openalex_id else {
            return Ok(None);
        };
        let mut url = Url::parse(&format!("https://api.openalex.org/works/{id}")).unwrap();
        self.add_mailto(&mut url);
        let value = self.get_json(&url, "openalex").await?;
        Ok(value
            .pointer("/best_oa_location/pdf_url")
            .or_else(|| value.pointer("/primary_location/pdf_url"))
            .and_then(Value::as_str)
            .map(|url| FullTextLocation {
                url: url.to_string(),
                source: "openalex".to_string(),
                status: "openalex_oa_pdf".to_string(),
            }))
    }
}

pub(crate) fn openalex_filter(input: &AcademicSearchInput) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(from) = input.year_from {
        parts.push(format!("from_publication_date:{from}-01-01"));
    }
    if let Some(to) = input.year_to {
        parts.push(format!("to_publication_date:{to}-12-31"));
    }
    if input.open_access_only.unwrap_or(false) {
        parts.push("is_oa:true".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

pub(crate) fn openalex_sort(input: &AcademicSearchInput) -> Option<&'static str> {
    if sort_is(input.sort_by.as_deref(), "citations") {
        Some("cited_by_count:desc")
    } else if sort_is(input.sort_by.as_deref(), "date")
        && (input.year_from.is_some() || input.year_to.is_some())
    {
        Some("publication_date:desc")
    } else {
        None
    }
}

fn is_transient_gateway_status(status: u16) -> bool {
    matches!(status, 502..=504)
}

fn openalex_transient_delay(attempt: usize) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        _ => Duration::from_secs(3),
    }
}

pub(crate) fn parse_openalex_work(value: &Value) -> AcademicPaper {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let id = value.get("id").and_then(Value::as_str).map(str::to_string);
    let doi = value
        .get("doi")
        .and_then(Value::as_str)
        .map(|s| s.trim_start_matches("https://doi.org/").to_string());
    let pdf_url = value
        .pointer("/best_oa_location/pdf_url")
        .or_else(|| value.pointer("/primary_location/pdf_url"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut paper = AcademicPaper {
        id: doi
            .clone()
            .or_else(|| id.clone())
            .unwrap_or_else(|| title.clone()),
        title,
        authors: value
            .get("authorships")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| {
                a.pointer("/author/display_name")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect(),
        year: value
            .get("publication_year")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        venue: value
            .pointer("/primary_location/source/display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        doi,
        openalex_id: id.clone(),
        url: value.get("id").and_then(Value::as_str).map(str::to_string),
        pdf_url,
        citation_count: as_u32(value.get("cited_by_count").and_then(Value::as_u64)),
        open_access: value.pointer("/open_access/is_oa").and_then(Value::as_bool),
        license: value
            .pointer("/best_oa_location/license")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    if let Some(abstract_text) = value.get("abstract_inverted_index") {
        paper.abstract_text = Some(openalex_abstract(abstract_text));
    }
    if let Some(url) = &paper.url {
        paper
            .sources
            .push(source(url.clone(), "openalex", Some(paper.title.clone())));
    }
    if let Some(refs) = value.get("referenced_works").and_then(Value::as_array) {
        paper.reference_count = Some(refs.len() as u32);
        for reference in refs {
            if let Some(url) = reference.as_str() {
                paper
                    .sources
                    .push(source(url.to_string(), "openalex_reference", None));
            }
        }
    }
    paper
}

pub(crate) fn without_openalex_reference_sources(mut paper: AcademicPaper) -> AcademicPaper {
    paper
        .sources
        .retain(|source| source.provider.as_ref() != "openalex_reference");
    paper
}
