use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    pub provider: Cow<'static, str>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub published_date: Option<String>,
    /// Inline source content from the `resolve_content` pipeline.
    /// `None` → field absent from JSON (`include_content=false` path, backward-compat).
    /// `Some` → non-empty string: structured markdown or a deterministic failure note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl Source {
    pub fn new(url: impl Into<String>, provider: impl Into<Cow<'static, str>>) -> Self {
        Self {
            url: url.into(),
            provider: provider.into(),
            title: None,
            description: None,
            published_date: None,
            content: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_published_date(mut self, published_date: impl Into<String>) -> Self {
        self.published_date = Some(published_date.into());
        self
    }
}

pub fn merge_sources(primary: Vec<Source>, secondary: Vec<Source>) -> Vec<Source> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for source in primary.into_iter().chain(secondary) {
        if source.url.trim().is_empty() {
            continue;
        }
        if seen.insert(source.url.clone()) {
            merged.push(source);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_content_none_omitted_from_json() {
        let source = Source::new("https://example.com", "tavily");
        let value = serde_json::to_value(&source).unwrap();
        // D-05: None must produce NO key, not "content": null.
        assert!(value.get("content").is_none());
    }

    #[test]
    fn source_content_some_appears_in_json() {
        let mut source = Source::new("https://example.com", "tavily");
        source.content = Some("hello".to_string());
        let value = serde_json::to_value(&source).unwrap();
        assert_eq!(value["content"], "hello");
        assert!(!value["content"].as_str().unwrap().is_empty());
    }
}
