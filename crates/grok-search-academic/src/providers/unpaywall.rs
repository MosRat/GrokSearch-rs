use async_trait::async_trait;
use grok_search_net::http::get_json;
use grok_search_provider_core::{AcademicProvider, FullTextLocation};
use grok_search_types::{AcademicPaper, Result};
use reqwest::header::USER_AGENT;
use serde_json::Value;
use url::Url;

use crate::service::UA;

#[derive(Clone)]
pub(crate) struct UnpaywallProvider {
    client: reqwest::Client,
    email: Option<String>,
}

impl UnpaywallProvider {
    pub(crate) fn new(client: reqwest::Client, email: Option<String>) -> Self {
        Self { client, email }
    }
}

#[async_trait]
impl AcademicProvider for UnpaywallProvider {
    fn name(&self) -> &'static str {
        "unpaywall"
    }

    async fn resolve_fulltext(&self, paper: &AcademicPaper) -> Result<Option<FullTextLocation>> {
        let (Some(email), Some(doi)) = (&self.email, &paper.doi) else {
            return Ok(None);
        };
        let mut url = Url::parse(&format!("https://api.unpaywall.org/v2/{doi}")).unwrap();
        url.query_pairs_mut().append_pair("email", email);
        let value = get_json(&self.client, url.as_str(), &[(USER_AGENT, UA)], "unpaywall").await?;
        Ok(value
            .pointer("/best_oa_location/url_for_pdf")
            .or_else(|| value.pointer("/best_oa_location/url"))
            .and_then(Value::as_str)
            .map(|url| FullTextLocation {
                url: url.to_string(),
                source: "unpaywall".to_string(),
                status: "unpaywall_oa_pdf".to_string(),
            }))
    }
}
