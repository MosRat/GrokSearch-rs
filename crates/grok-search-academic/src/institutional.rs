use std::sync::Arc;
use std::time::Duration;

use grok_search_config::Config;
use grok_search_net::http::{
    build_client_direct_with_options, build_client_with_proxy_options, ClientOptions,
};
use grok_search_net::proxy::{discover_all_candidates, ProxyCandidate};
use grok_search_net::url_policy::{url_is_private_or_local, validate_http_url};
use grok_search_pdf::{download_pdf_bytes_with_options_limited, PdfDownloadOptions};
use grok_search_provider_core::FullTextLocation;
use grok_search_types::{AcademicPaper, GrokSearchError, Result};
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, USER_AGENT};
use tokio::sync::OnceCell;
use url::Url;

const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/126 Safari/537.36";
const IEEE_PROBE_ARNUMBER: &str = "8262806";
const ACM_PROBE_DOI: &str = "10.1145/3544548.3581390";

#[derive(Clone)]
pub(crate) struct InstitutionalAccessManager {
    config: Config,
    state: Arc<OnceCell<InstitutionalState>>,
}

#[derive(Clone)]
struct InstitutionalState {
    enabled: bool,
    ieee: Option<InstitutionalSession>,
    acm: Option<InstitutionalSession>,
    detail: String,
}

#[derive(Clone)]
struct InstitutionalSession {
    client: reqwest::Client,
    route: RouteInfo,
}

#[derive(Clone)]
struct RouteInfo {
    kind: String,
    source: String,
    proxy_url_redacted: Option<String>,
}

struct CandidateRoute {
    client: reqwest::Client,
    info: RouteInfo,
}

impl InstitutionalAccessManager {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            config,
            state: Arc::new(OnceCell::new()),
        }
    }

    pub(crate) fn warm(&self) {
        if !self.config.academic_institutional_enabled || !self.config.academic_institutional_probe
        {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.state().await;
        });
    }

    pub(crate) async fn resolve_url_location(&self, url: &str) -> Option<FullTextLocation> {
        if !self.config.academic_institutional_enabled {
            return None;
        }
        let state = self.state().await;
        if !state.enabled {
            return None;
        }
        if state.acm.is_some() {
            if let Some(doi) = extract_acm_doi(url) {
                return Some(FullTextLocation {
                    url: acm_pdf_url(&doi),
                    source: "acm_institutional".to_string(),
                    status: "institutional_pdf".to_string(),
                });
            }
        }
        if state.ieee.is_some() {
            if let Some(arnumber) = extract_ieee_arnumber(url) {
                return Some(FullTextLocation {
                    url: ieee_pdf_url(&arnumber),
                    source: "ieee_institutional".to_string(),
                    status: "institutional_pdf".to_string(),
                });
            }
        }
        None
    }

    pub(crate) async fn resolve_locations(&self, paper: &AcademicPaper) -> Vec<FullTextLocation> {
        if !self.config.academic_institutional_enabled {
            return Vec::new();
        }
        let state = self.state().await;
        if !state.enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        if state.acm.is_some() {
            if let Some(doi) = acm_doi_for_paper(paper) {
                out.push(FullTextLocation {
                    url: acm_pdf_url(&doi),
                    source: "acm_institutional".to_string(),
                    status: "institutional_pdf".to_string(),
                });
            }
        }
        if state.ieee.is_some() {
            if let Some(arnumber) = ieee_arnumber_for_paper(paper, state.ieee.as_ref()).await {
                out.push(FullTextLocation {
                    url: ieee_pdf_url(&arnumber),
                    source: "ieee_institutional".to_string(),
                    status: "institutional_pdf".to_string(),
                });
            }
        }
        out
    }

    pub(crate) async fn download_pdf(
        &self,
        location: &FullTextLocation,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        let state = self.state().await;
        let (session, warmup_url) = match location.source.as_str() {
            "acm_institutional" => (state.acm.as_ref(), acm_detail_url_from_pdf(&location.url)),
            "ieee_institutional" => (
                state.ieee.as_ref(),
                extract_ieee_arnumber(&location.url).map(|id| ieee_detail_url(&id)),
            ),
            _ => (None, None),
        };
        let Some(session) = session else {
            return Err(GrokSearchError::Provider(format!(
                "{} institutional access is unavailable",
                location.source
            )));
        };
        let parsed = validate_http_url(&location.url)?;
        let private_or_local = url_is_private_or_local(&parsed);
        if !private_or_local && parsed.scheme() != "https" {
            return Err(GrokSearchError::InvalidParams(
                "institutional IEEE/ACM public PDF URLs must use https".to_string(),
            ));
        }
        let client = if private_or_local {
            build_client_direct_with_options(
                self.config.timeout,
                ClientOptions {
                    cookies: true,
                    accept_invalid_certs: true,
                },
            )?
        } else {
            session.client.clone()
        };
        let options = PdfDownloadOptions {
            label: location.source.as_str(),
            warmup_url: warmup_url.as_deref(),
            headers: &browser_pdf_headers(),
        };
        download_pdf_bytes_with_options_limited(
            &client,
            &location.url,
            max_bytes,
            options,
            self.config.max_response_bytes,
        )
        .await
    }

    pub(crate) async fn diagnostics(&self, live: bool) -> serde_json::Value {
        if !self.config.academic_institutional_enabled {
            return serde_json::json!({
                "enabled": false,
                "status": "disabled",
                "detail": "disabled by configuration",
                "ieee": source_diag(None),
                "acm": source_diag(None),
            });
        }
        if live {
            let state = self.state().await;
            return state.to_json(true);
        }
        match self.state.get() {
            Some(state) => state.to_json(true),
            None => serde_json::json!({
                "enabled": true,
                "status": "pending",
                "detail": "institutional access probe has not completed",
                "ieee": source_diag(None),
                "acm": source_diag(None),
            }),
        }
    }

    async fn state(&self) -> &InstitutionalState {
        self.state
            .get_or_init(|| async { self.probe().await })
            .await
    }

    async fn probe(&self) -> InstitutionalState {
        if !self.config.academic_institutional_enabled {
            return InstitutionalState::disabled("disabled by configuration");
        }
        if !self.config.academic_institutional_probe {
            return InstitutionalState::disabled("probe disabled by configuration");
        }
        let routes = self.routes();
        let mut ieee = None;
        let mut acm = None;
        let mut failures = Vec::new();
        for route in routes {
            if ieee.is_none() {
                match probe_ieee(&route.client).await {
                    Ok(()) => {
                        ieee = Some(InstitutionalSession {
                            client: route.client.clone(),
                            route: route.info.clone(),
                        });
                    }
                    Err(err) => failures.push(format!("ieee {}: {err}", route.info.label())),
                }
            }
            if acm.is_none() {
                match probe_acm(&route.client).await {
                    Ok(()) => {
                        acm = Some(InstitutionalSession {
                            client: route.client.clone(),
                            route: route.info.clone(),
                        });
                    }
                    Err(err) => failures.push(format!("acm {}: {err}", route.info.label())),
                }
            }
            if ieee.is_some() && acm.is_some() {
                break;
            }
        }
        let enabled = ieee.is_some() || acm.is_some();
        InstitutionalState {
            enabled,
            ieee,
            acm,
            detail: if enabled {
                "institutional access available".to_string()
            } else if failures.is_empty() {
                "no institutional access candidates".to_string()
            } else {
                format!("institutional access unavailable ({})", failures.join("; "))
            },
        }
    }

    fn routes(&self) -> Vec<CandidateRoute> {
        let options = ClientOptions {
            cookies: true,
            accept_invalid_certs: self.config.academic_institutional_accept_invalid_certs,
        };
        let mut routes = Vec::new();
        if let Ok(client) = build_client_direct_with_options(self.config.timeout, options) {
            routes.push(CandidateRoute {
                client,
                info: RouteInfo {
                    kind: "direct".to_string(),
                    source: "direct".to_string(),
                    proxy_url_redacted: None,
                },
            });
        }
        for candidate in discover_all_candidates() {
            if let Some(route) = build_proxy_route(self.config.timeout, options, candidate) {
                routes.push(route);
            }
        }
        routes
    }
}

impl InstitutionalState {
    fn disabled(detail: impl Into<String>) -> Self {
        Self {
            enabled: false,
            ieee: None,
            acm: None,
            detail: detail.into(),
        }
    }

    fn to_json(&self, configured: bool) -> serde_json::Value {
        serde_json::json!({
            "enabled": configured,
            "status": if self.enabled { "enabled" } else { "disabled" },
            "detail": self.detail,
            "ieee": source_diag(self.ieee.as_ref()),
            "acm": source_diag(self.acm.as_ref()),
        })
    }
}

impl RouteInfo {
    fn label(&self) -> String {
        match &self.proxy_url_redacted {
            Some(url) => format!("{}:{} {url}", self.kind, self.source),
            None => self.kind.clone(),
        }
    }
}

fn build_proxy_route(
    timeout: Duration,
    options: ClientOptions,
    candidate: ProxyCandidate,
) -> Option<CandidateRoute> {
    let client = build_client_with_proxy_options(timeout, candidate.url(), options).ok()?;
    Some(CandidateRoute {
        client,
        info: RouteInfo {
            kind: "proxy".to_string(),
            source: candidate.source().to_string(),
            proxy_url_redacted: Some(candidate.redacted_url()),
        },
    })
}

fn source_diag(session: Option<&InstitutionalSession>) -> serde_json::Value {
    match session {
        Some(session) => serde_json::json!({
            "available": true,
            "route": session.route.kind,
            "source": session.route.source,
            "proxy_url": session.route.proxy_url_redacted,
        }),
        None => serde_json::json!({
            "available": false,
            "route": serde_json::Value::Null,
            "source": serde_json::Value::Null,
            "proxy_url": serde_json::Value::Null,
        }),
    }
}

async fn probe_ieee(client: &reqwest::Client) -> Result<()> {
    let url = ieee_pdf_url(IEEE_PROBE_ARNUMBER);
    let location = FullTextLocation {
        url,
        source: "ieee_institutional".to_string(),
        status: "institutional_pdf".to_string(),
    };
    let options = PdfDownloadOptions {
        label: "ieee institutional probe",
        warmup_url: Some(&ieee_detail_url(IEEE_PROBE_ARNUMBER)),
        headers: &browser_pdf_headers(),
    };
    download_pdf_bytes_with_options_limited(
        client,
        &location.url,
        5 * 1024 * 1024,
        options,
        5 * 1024 * 1024,
    )
    .await
    .map(|_| ())
}

async fn probe_acm(client: &reqwest::Client) -> Result<()> {
    let url = acm_pdf_url(ACM_PROBE_DOI);
    let options = PdfDownloadOptions {
        label: "acm institutional probe",
        warmup_url: Some(&acm_detail_url(ACM_PROBE_DOI)),
        headers: &browser_pdf_headers(),
    };
    download_pdf_bytes_with_options_limited(
        client,
        &url,
        10 * 1024 * 1024,
        options,
        10 * 1024 * 1024,
    )
    .await
    .map(|_| ())
}

pub(crate) fn extract_ieee_arnumber(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    let host = url.host_str().unwrap_or_default();
    if !host.ends_with("ieeexplore.ieee.org") {
        return None;
    }
    if let Some(id) = url
        .query_pairs()
        .find(|(key, _)| key == "arnumber")
        .map(|(_, value)| value.into_owned())
        .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
    {
        return Some(id);
    }
    let path = url.path();
    for prefix in ["/document/", "/abstract/document/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let id: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

pub(crate) fn extract_acm_doi(value: &str) -> Option<String> {
    let normalized = value.trim().trim_start_matches("doi:").trim();
    if normalized.starts_with("10.1145/") {
        return Some(trim_doi_suffix(normalized));
    }
    if let Ok(url) = Url::parse(normalized) {
        if url
            .host_str()
            .is_some_and(|host| host.ends_with("dl.acm.org"))
        {
            for prefix in ["/doi/pdf/", "/doi/abs/", "/doi/"] {
                if let Some(rest) = url.path().strip_prefix(prefix) {
                    let doi = trim_doi_suffix(rest);
                    if doi.starts_with("10.1145/") {
                        return Some(doi);
                    }
                }
            }
        }
    }
    None
}

fn acm_doi_for_paper(paper: &AcademicPaper) -> Option<String> {
    paper
        .doi
        .as_deref()
        .and_then(extract_acm_doi)
        .or_else(|| paper.url.as_deref().and_then(extract_acm_doi))
        .or_else(|| {
            paper
                .sources
                .iter()
                .find_map(|source| extract_acm_doi(&source.url))
        })
}

async fn ieee_arnumber_for_paper(
    paper: &AcademicPaper,
    session: Option<&InstitutionalSession>,
) -> Option<String> {
    if let Some(id) = paper
        .url
        .as_deref()
        .and_then(extract_ieee_arnumber)
        .or_else(|| {
            paper
                .sources
                .iter()
                .find_map(|source| extract_ieee_arnumber(&source.url))
        })
    {
        return Some(id);
    }
    let doi = paper.doi.as_deref()?;
    let session = session?;
    resolve_ieee_arnumber_from_doi(&session.client, doi).await
}

async fn resolve_ieee_arnumber_from_doi(client: &reqwest::Client, doi: &str) -> Option<String> {
    let response = client
        .get(format!("https://doi.org/{doi}"))
        .header(USER_AGENT, BROWSER_UA)
        .send()
        .await
        .ok()?;
    extract_ieee_arnumber(response.url().as_str())
}

fn ieee_detail_url(arnumber: &str) -> String {
    format!("https://ieeexplore.ieee.org/document/{arnumber}")
}

fn ieee_pdf_url(arnumber: &str) -> String {
    format!("https://ieeexplore.ieee.org/stampPDF/getPDF.jsp?tp=&arnumber={arnumber}")
}

fn acm_detail_url(doi: &str) -> String {
    format!("https://dl.acm.org/doi/{doi}")
}

fn acm_pdf_url(doi: &str) -> String {
    format!("https://dl.acm.org/doi/pdf/{doi}")
}

fn acm_detail_url_from_pdf(url: &str) -> Option<String> {
    extract_acm_doi(url).map(|doi| acm_detail_url(&doi))
}

fn trim_doi_suffix(value: &str) -> String {
    value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches(".pdf")
        .to_string()
}

fn browser_pdf_headers() -> Vec<(reqwest::header::HeaderName, &'static str)> {
    vec![
        (USER_AGENT, BROWSER_UA),
        (ACCEPT, "application/pdf,*/*"),
        (ACCEPT_ENCODING, "identity"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use grok_search_types::Source;

    #[test]
    fn extracts_ieee_arnumber_from_common_urls() {
        assert_eq!(
            extract_ieee_arnumber("https://ieeexplore.ieee.org/document/8262806").as_deref(),
            Some("8262806")
        );
        assert_eq!(
            extract_ieee_arnumber(
                "https://ieeexplore.ieee.org/stamp/stamp.jsp?tp=&arnumber=7780459"
            )
            .as_deref(),
            Some("7780459")
        );
        assert_eq!(
            extract_ieee_arnumber(
                "https://ieeexplore.ieee.org/stampPDF/getPDF.jsp?tp=&arnumber=7780460"
            )
            .as_deref(),
            Some("7780460")
        );
    }

    #[test]
    fn extracts_acm_doi_from_common_inputs() {
        assert_eq!(
            extract_acm_doi("10.1145/3544548.3581390").as_deref(),
            Some("10.1145/3544548.3581390")
        );
        assert_eq!(
            extract_acm_doi("https://dl.acm.org/doi/10.1145/3544548.3581390").as_deref(),
            Some("10.1145/3544548.3581390")
        );
        assert_eq!(
            extract_acm_doi("https://dl.acm.org/doi/pdf/10.1145/3544548.3581390").as_deref(),
            Some("10.1145/3544548.3581390")
        );
    }

    #[test]
    fn acm_doi_ignores_non_acm_doi() {
        assert!(extract_acm_doi("10.1109/5.771073").is_none());
    }

    #[tokio::test]
    async fn institutional_locations_use_available_sources() {
        let state = InstitutionalState {
            enabled: true,
            ieee: None,
            acm: Some(InstitutionalSession {
                client: reqwest::Client::new(),
                route: RouteInfo {
                    kind: "direct".into(),
                    source: "direct".into(),
                    proxy_url_redacted: None,
                },
            }),
            detail: "ok".into(),
        };
        let manager = InstitutionalAccessManager {
            config: Config::from_env_map(Vec::<(String, String)>::new()),
            state: Arc::new(OnceCell::const_new_with(state)),
        };
        let paper = AcademicPaper {
            doi: Some("10.1145/3544548.3581390".into()),
            sources: vec![Source::new(
                "https://dl.acm.org/doi/10.1145/3544548.3581390",
                "acm",
            )],
            ..Default::default()
        };
        let locations = manager.resolve_locations(&paper).await;
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].source, "acm_institutional");
        assert_eq!(
            locations[0].url,
            "https://dl.acm.org/doi/pdf/10.1145/3544548.3581390"
        );
    }

    #[test]
    fn ieee_arnumber_does_not_guess_from_plain_doi_digits() {
        assert!(extract_ieee_arnumber("10.1145/3544548.3581390").is_none());
        assert!(extract_ieee_arnumber("10.1109/5.771073").is_none());
    }

    #[tokio::test]
    async fn institutional_private_http_pdf_is_allowed() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let body = b"%PDF-1.7\n";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("headers");
            stream.write_all(body).expect("body");
        });

        let state = InstitutionalState {
            enabled: true,
            ieee: Some(InstitutionalSession {
                client: reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .expect("client"),
                route: RouteInfo {
                    kind: "direct".into(),
                    source: "direct".into(),
                    proxy_url_redacted: None,
                },
            }),
            acm: None,
            detail: "ok".into(),
        };
        let manager = InstitutionalAccessManager {
            config: Config::from_env_map(Vec::<(String, String)>::new()),
            state: Arc::new(OnceCell::const_new_with(state)),
        };
        let bytes = manager
            .download_pdf(
                &FullTextLocation {
                    url,
                    source: "ieee_institutional".into(),
                    status: "institutional_pdf".into(),
                },
                1024,
            )
            .await
            .expect("private HTTP institutional PDF");
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn institutional_invalid_cert_client_option_builds() {
        let client = build_client_direct_with_options(
            Duration::from_secs(5),
            ClientOptions {
                cookies: true,
                accept_invalid_certs: true,
            },
        )
        .expect("institutional client with invalid cert option");
        drop(client);
    }
}
