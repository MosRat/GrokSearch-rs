use async_trait::async_trait;
use grok_search_net::http::get_json;
use grok_search_provider_core::{AcademicIdentifier as Identifier, AcademicProvider};
use grok_search_types::{AcademicPaper, AcademicSearchInput, Result};
use reqwest::header::USER_AGENT;
use serde_json::Value;
use url::Url;

use super::sort_is;
use crate::service::{as_u32, source, UA};

#[derive(Clone)]
pub(crate) struct CrossrefProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl CrossrefProvider {
    pub(crate) fn new(client: reqwest::Client, email: Option<String>) -> Self {
        Self { client, email }
    }
}

#[async_trait]
impl AcademicProvider for CrossrefProvider {
    fn name(&self) -> &'static str {
        "crossref"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://api.crossref.org/works").unwrap();
        url.query_pairs_mut()
            .append_pair("query.bibliographic", &input.query)
            .append_pair("rows", &limit.to_string());
        if let Some(filter) = crossref_filter(input.year_from, input.year_to) {
            url.query_pairs_mut().append_pair("filter", &filter);
        }
        if let Some((sort, order)) = crossref_sort(input.sort_by.as_deref()) {
            url.query_pairs_mut()
                .append_pair("sort", sort)
                .append_pair("order", order);
        }
        if let Some(email) = &self.email {
            url.query_pairs_mut().append_pair("mailto", email);
        }
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "crossref").await?;
        Ok(value
            .pointer("/message/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(parse_crossref_work)
            .collect())
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        let Identifier::Doi(doi) = identifier else {
            return Ok(None);
        };
        let url = format!("https://api.crossref.org/works/{doi}");
        let value = get_json(&self.client, &url, &[(USER_AGENT, UA)], "crossref").await?;
        Ok(value.get("message").map(parse_crossref_work))
    }
}

pub(crate) fn parse_crossref_work(value: &Value) -> AcademicPaper {
    let title = value
        .get("title")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let doi = value.get("DOI").and_then(Value::as_str).map(str::to_string);
    let url = value.get("URL").and_then(Value::as_str).map(str::to_string);
    let mut paper = AcademicPaper {
        id: doi.clone().unwrap_or_else(|| title.clone()),
        title,
        authors: value
            .get("author")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| {
                let given = a.get("given").and_then(Value::as_str).unwrap_or("");
                let family = a.get("family").and_then(Value::as_str).unwrap_or("");
                format!("{given} {family}").trim().to_string()
            })
            .filter(|s| !s.is_empty())
            .collect(),
        year: value
            .pointer("/published-print/date-parts/0/0")
            .or_else(|| value.pointer("/published-online/date-parts/0/0"))
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        venue: value
            .get("container-title")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_string),
        abstract_text: value
            .get("abstract")
            .and_then(Value::as_str)
            .map(str::to_string),
        doi,
        url: url.clone(),
        reference_count: as_u32(value.get("reference-count").and_then(Value::as_u64)),
        license: value
            .get("license")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|v| v.get("URL"))
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };
    if let Some(url) = url {
        paper
            .sources
            .push(source(url, "crossref", Some(paper.title.clone())));
    }
    paper
}

pub(crate) fn crossref_filter(year_from: Option<u32>, year_to: Option<u32>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(from) = year_from {
        parts.push(format!("from-pub-date:{from}-01-01"));
    }
    if let Some(to) = year_to {
        parts.push(format!("until-pub-date:{to}-12-31"));
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

pub(crate) fn crossref_sort(sort_by: Option<&str>) -> Option<(&'static str, &'static str)> {
    if sort_is(sort_by, "citations") {
        Some(("is-referenced-by-count", "desc"))
    } else if sort_is(sort_by, "date") {
        Some(("published", "desc"))
    } else {
        None
    }
}
