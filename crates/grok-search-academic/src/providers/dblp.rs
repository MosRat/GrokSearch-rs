use async_trait::async_trait;
use grok_search_net::http::get_json;
use grok_search_provider_core::{AcademicIdentifier as Identifier, AcademicProvider};
use grok_search_types::{AcademicPaper, AcademicSearchInput, Result};
use reqwest::header::USER_AGENT;
use serde_json::Value;
use url::Url;

use super::clean_title;
use crate::service::{source, UA};

#[derive(Clone)]
pub(crate) struct DblpProvider {
    client: reqwest::Client,
}

impl DblpProvider {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl AcademicProvider for DblpProvider {
    fn name(&self) -> &'static str {
        "dblp"
    }

    async fn search(
        &self,
        input: &AcademicSearchInput,
        limit: usize,
    ) -> Result<Vec<AcademicPaper>> {
        let mut url = Url::parse("https://dblp.org/search/publ/api").unwrap();
        url.query_pairs_mut()
            .append_pair("q", &input.query)
            .append_pair("format", "json")
            .append_pair("h", &limit.to_string());
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "dblp").await?;
        Ok(parse_dblp_search(&value))
    }

    async fn get(&self, identifier: &Identifier) -> Result<Option<AcademicPaper>> {
        match identifier {
            Identifier::Dblp(key) | Identifier::Url(key) | Identifier::Query(key) => {
                let input = AcademicSearchInput {
                    query: key.clone(),
                    ..Default::default()
                };
                Ok(self.search(&input, 1).await?.into_iter().next())
            }
            _ => Ok(None),
        }
    }
}

pub(crate) fn parse_dblp_search(value: &Value) -> Vec<AcademicPaper> {
    value
        .pointer("/result/hits/hit")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hit| {
            let info = hit.get("info")?;
            let title = info.get("title").and_then(Value::as_str)?.to_string();
            let url = info.get("url").and_then(Value::as_str).map(str::to_string);
            let doi = info.get("doi").and_then(Value::as_str).map(str::to_string);
            let authors = parse_dblp_authors(info.pointer("/authors/author"));
            let year = info
                .get("year")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok());
            let venue = info
                .get("venue")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut paper = AcademicPaper {
                id: doi
                    .clone()
                    .or_else(|| url.clone())
                    .unwrap_or_else(|| title.clone()),
                title: clean_title(&title),
                authors,
                year,
                venue,
                doi,
                url: url.clone(),
                ..Default::default()
            };
            if let Some(url) = url {
                paper
                    .sources
                    .push(source(url, "dblp", Some(paper.title.clone())));
            }
            Some(paper)
        })
        .collect()
}

fn parse_dblp_authors(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.get("text").or(Some(v)).and_then(Value::as_str))
            .map(str::to_string)
            .collect(),
        Some(Value::Object(_)) => value
            .and_then(|v| v.get("text").or(Some(v)).and_then(Value::as_str))
            .map(|s| vec![s.to_string()])
            .unwrap_or_default(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}
