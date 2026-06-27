use grok_search_types::model::search::SearchFilters;
use grok_search_types::model::source::Source;

pub(crate) fn filter_sources_by_domains(
    sources: Vec<Source>,
    filters: &SearchFilters,
) -> Vec<Source> {
    let include = DomainSet::new(&filters.include_domains);
    let exclude = DomainSet::new(&filters.exclude_domains);
    if include.is_empty() && exclude.is_empty() {
        return sources;
    }
    sources
        .into_iter()
        .filter(|source| source_allowed_by_domains(&source.url, &include, &exclude))
        .collect()
}

fn source_allowed_by_domains(url: &str, include: &DomainSet, exclude: &DomainSet) -> bool {
    let Some(host) = host_for_match(url) else {
        return include.is_empty();
    };
    (include.is_empty() || include.matches(&host)) && !exclude.matches(&host)
}

#[derive(Debug, Clone)]
struct DomainSet {
    domains: Vec<String>,
}

impl DomainSet {
    fn new(raw: &[String]) -> Self {
        let domains = raw
            .iter()
            .filter_map(|value| normalize_domain(value))
            .collect();
        Self { domains }
    }

    fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }

    fn matches(&self, host: &str) -> bool {
        self.domains
            .iter()
            .any(|domain| host == domain || host.ends_with(&format!(".{domain}")))
    }
}

fn host_for_match(raw: &str) -> Option<String> {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(normalize_host))
        .or_else(|| normalize_domain(raw))
}

fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(trimmed) {
        return url.host_str().map(normalize_host);
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    let host = normalize_host(host);
    (!host.is_empty()).then_some(host)
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_end_matches('.')
        .trim_start_matches("www.")
        .to_ascii_lowercase()
}
