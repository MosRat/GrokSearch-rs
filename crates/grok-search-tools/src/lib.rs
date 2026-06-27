use grok_search_service::SearchService;
use grok_search_types::model::tool::WebSearchInput;
use grok_search_types::AcademicSearchInput;
use grok_search_types::{GrokSearchError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Map<String, Value>,
}

pub fn tools() -> Vec<ToolSpec> {
    tools_list_json()["tools"]
        .as_array()
        .expect("tools_list is an array")
        .iter()
        .map(tool_from_value)
        .collect()
}

fn tool_from_value(value: &Value) -> ToolSpec {
    ToolSpec {
        name: value["name"].as_str().expect("tool name").to_string(),
        description: value["description"]
            .as_str()
            .expect("tool description")
            .to_string(),
        input_schema: value["inputSchema"]
            .as_object()
            .expect("input schema")
            .clone(),
    }
}

pub async fn invoke_tool(service: &SearchService, name: &str, args: Value) -> Result<Value> {
    match name {
        "doctor" => {
            let params: DoctorParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            Ok(service
                .doctor_with_options(params.verbose.unwrap_or(false))
                .await)
        }
        "web_search" => {
            let params: WebSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.query.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "web_search.query is required".into(),
                ));
            }
            let output = service.web_search(params.into()).await?;
            serialize_output(output, "serialize output")
        }
        "get_sources" => {
            let params: GetSourcesParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .get_sources(
                    &params.session_id,
                    params.offset.unwrap_or(0),
                    params.limit.filter(|value| *value > 0),
                )
                .await?;
            serialize_output(output, "serialize sources")
        }
        "web_fetch" => {
            let params: WebFetchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .web_fetch(&params.url, params.max_chars.filter(|value| *value > 0))
                .await?;
            serialize_output(output, "serialize fetch")
        }
        "web_map" => {
            let params: WebMapParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let max_results = params.max_results.unwrap_or(10);
            if !(1..=50).contains(&max_results) {
                return Err(GrokSearchError::InvalidParams(
                    "web_map.max_results must be between 1 and 50".to_string(),
                ));
            }
            let sources = service.web_map(&params.url, max_results).await?;
            let mapped_sources: Vec<Value> = sources
                .iter()
                .map(|source| json!({ "url": &source.url, "provider": &source.provider }))
                .collect();
            Ok(json!({
                "url": params.url,
                "sources_count": mapped_sources.len(),
                "sources": mapped_sources
            }))
        }
        "academic_search" => {
            let params: AcademicSearchParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.query.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_search.query is required".into(),
                ));
            }
            let output = service.academic_search(params.into()).await?;
            serialize_output(output, "serialize academic search")
        }
        "academic_get" => {
            let params: AcademicGetParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.trim().is_empty() {
                return Err(GrokSearchError::InvalidParams(
                    "academic_get.identifier is required".into(),
                ));
            }
            let output = service
                .academic_get(
                    &params.identifier,
                    params.include_citations.unwrap_or(false),
                    params.include_open_access.unwrap_or(true),
                )
                .await?;
            serialize_output(output, "serialize academic get")
        }
        "academic_citations" => {
            let params: AcademicCitationsParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            let output = service
                .academic_citations(&params.identifier, params.limit.filter(|value| *value > 0))
                .await?;
            serialize_output(output, "serialize academic citations")
        }
        "academic_read" => {
            let params: AcademicReadParams = serde_json::from_value(args)
                .map_err(|err| GrokSearchError::InvalidParams(err.to_string()))?;
            if params.identifier.as_deref().unwrap_or("").trim().is_empty()
                && params.url.as_deref().unwrap_or("").trim().is_empty()
            {
                return Err(GrokSearchError::InvalidParams(
                    "academic_read requires identifier or url".into(),
                ));
            }
            let output = service
                .academic_read(
                    params.identifier,
                    params.url,
                    params.max_chars,
                    params.output_format,
                )
                .await?;
            serialize_output(output, "serialize academic read")
        }
        _ => Err(GrokSearchError::NotFound(format!("unknown tool: {name}"))),
    }
}

pub fn serialize_output<T: serde::Serialize>(output: T, context: &str) -> Result<Value> {
    serde_json::to_value(output).map_err(|err| GrokSearchError::Parse(format!("{context}: {err}")))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    pub query: String,
    pub platform: Option<String>,
    pub model: Option<String>,
    pub extra_sources: Option<usize>,
    pub recency_days: Option<u32>,
    #[serde(default)]
    pub include_domains: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
    pub include_content: Option<bool>,
    pub response_format: Option<String>,
}

impl From<WebSearchParams> for WebSearchInput {
    fn from(params: WebSearchParams) -> Self {
        Self {
            query: params.query,
            platform: params.platform,
            model: params.model,
            extra_sources: params.extra_sources,
            recency_days: params.recency_days.filter(|value| *value > 0),
            include_domains: params.include_domains,
            exclude_domains: params.exclude_domains,
            include_content: params.include_content,
            response_format: params.response_format,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetSourcesParams {
    pub session_id: String,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebFetchParams {
    pub url: String,
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebMapParams {
    pub url: String,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DoctorParams {
    pub verbose: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicSearchParams {
    pub query: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub search_mode: Option<String>,
    pub sort_by: Option<String>,
    pub max_results: Option<usize>,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub open_access_only: Option<bool>,
    pub include_abstract: Option<bool>,
    pub include_citations: Option<bool>,
}

impl From<AcademicSearchParams> for AcademicSearchInput {
    fn from(params: AcademicSearchParams) -> Self {
        Self {
            query: params.query,
            sources: params.sources,
            search_mode: params.search_mode,
            sort_by: params.sort_by,
            max_results: params.max_results,
            year_from: params.year_from,
            year_to: params.year_to,
            open_access_only: params.open_access_only,
            include_abstract: params.include_abstract,
            include_citations: params.include_citations,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicGetParams {
    pub identifier: String,
    pub include_citations: Option<bool>,
    pub include_open_access: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicCitationsParams {
    pub identifier: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AcademicReadParams {
    pub identifier: Option<String>,
    pub url: Option<String>,
    pub max_chars: Option<usize>,
    pub output_format: Option<String>,
}

pub fn tools_list_json() -> Value {
    json!({
        "tools": [
            {
                "name": "web_search",
                "description": "Use for discovery 鈥?when you don't have a specific URL and need to find information, debug an error, research a topic, or track down an issue or news item. Returns an AI-synthesised answer plus a source list. By default the first few sources carry inline content (max_inline_sources, default 5); the rest are metadata-only 鈥?drill into any of them with web_fetch(url). The whole response is capped by a character budget; when truncated=true, trimmed sources carry a note telling you how to recover the full text via web_fetch or get_sources. Pass response_format=\"concise\" for answer + source metadata only. If you already know the exact page URL, use web_fetch instead.",
                "inputSchema": {
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "platform": { "type": "string" },
                        "model": { "type": "string" },
                        "extra_sources": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Optional supplemental source count. Tavily is primary; Firecrawl is fallback. If omitted, GROK_SEARCH_EXTRA_SOURCES is used."
                        },
                        "recency_days": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Restrict supplemental results to sources published within the last N days. Forwarded to Tavily as days+topic=news; also hinted to Grok prompt."
                        },
                        "include_domains": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Only return supplemental results from these domains. Tavily honors strictly; Grok receives as soft preference."
                        },
                        "exclude_domains": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Suppress supplemental results from these domains. Tavily honors strictly; Grok receives as soft instruction."
                        },
                        "include_content": {
                            "type": "boolean",
                            "default": true,
                            "description": "Inline source content via the resolve_content pipeline. Default true. Pass false to get summary + source-list only (legacy behavior, no content field in sources). Superseded by response_format when both are set."
                        },
                        "response_format": {
                            "type": "string",
                            "enum": ["concise", "detailed"],
                            "description": "concise = synthesized answer + source metadata only (smallest payload); detailed = inline source content, subject to the response budget. Takes precedence over include_content."
                        }
                    }
                }
            },
            {
                "name": "get_sources",
                "description": "Return cached sources from a previous web_search call by session_id. Use to re-examine sources already retrieved without issuing a new search 鈥?it reuses the prior session and runs no new search or fetch. Paginate with offset/limit: the response reports total_sources and, when more pages remain, next_offset to pass as the next offset.",
                "inputSchema": {
                    "type": "object",
                    "required": ["session_id"],
                    "properties": {
                        "session_id": { "type": "string" },
                        "offset": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 0,
                            "description": "Index of the first source to return. Use next_offset from the previous page to continue."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Max sources in this page. Omit to return all remaining sources (still subject to the response budget)."
                        }
                    }
                }
            },
            {
                "name": "web_fetch",
                "description": "Use when you already have a specific URL and want to read a single page in depth. GitHub issue/PR, StackOverflow (StackExchange), arXiv, and Wikipedia URLs are automatically parsed into structured, de-noised Markdown ready to feed an LLM; all other pages fall back to generic extraction. Returns {url, content, original_length, truncated, source_type, fallback_reason?}. If you don't have a URL yet and need to discover sources, use web_search instead.",
                "inputSchema": {
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": { "type": "string" },
                        "max_chars": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional character cap on returned content. Falls back to GROK_SEARCH_FETCH_MAX_CHARS, otherwise unlimited."
                        }
                    }
                }
            },
            {
                "name": "web_map",
                "description": "Map/discover URLs through Tavily Map.",
                "inputSchema": {
                    "type": "object",
                    "required": ["url"],
                    "properties": {
                        "url": { "type": "string" },
                        "max_results": { "type": "integer", "minimum": 1, "maximum": 50 }
                    }
                }
            },
            {
                "name": "doctor",
                "description": "Diagnostic probe: live connectivity check for Grok, Tavily, and Firecrawl backends, plus masked configuration. Use to verify the server is wired up and reachable.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "verbose": {
                            "type": "boolean",
                            "default": false,
                            "description": "Include detailed limits, logging status, provider wiring, and URL policy diagnostics."
                        }
                    }
                }
            },
            {
                "name": "academic_search",
                "description": "Search computer-science academic literature across dblp, Semantic Scholar, arXiv, OpenAlex, and Crossref. Results are deduplicated by DOI/arXiv/title and ranked with reciprocal rank fusion; OpenAlex/Crossref/Semantic metadata is used to enrich papers when available.",
                "inputSchema": {
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" },
                        "sources": {
                            "type": "array",
                            "items": { "type": "string", "enum": ["dblp", "semantic", "arxiv", "openalex", "crossref"] },
                            "description": "Selected sources. Defaults depend on search_mode; balanced uses dblp, Semantic Scholar, and arXiv as primary sources and enriches from OpenAlex/Crossref when possible."
                        },
                        "search_mode": {
                            "type": "string",
                            "enum": ["balanced", "broad", "precise"],
                            "default": "balanced",
                            "description": "balanced = stable CS discovery with metadata enrichment; broad = include all providers for maximum recall; precise = stricter title/query matching for known-paper lookups."
                        },
                        "sort_by": {
                            "type": "string",
                            "enum": ["relevance", "citations", "date"],
                            "default": "relevance",
                            "description": "Final ranking preference. relevance keeps query match first, citations boosts highly cited relevant papers, date boosts recent relevant papers. Provider APIs use matching native sort parameters when available."
                        },
                        "max_results": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 },
                        "year_from": { "type": "integer", "minimum": 1 },
                        "year_to": { "type": "integer", "minimum": 1 },
                        "open_access_only": { "type": "boolean" },
                        "include_abstract": { "type": "boolean", "default": true },
                        "include_citations": { "type": "boolean", "default": false }
                    }
                }
            },
            {
                "name": "academic_get",
                "description": "Resolve one academic paper by DOI, arXiv ID/URL, Semantic Scholar paperId, OpenAlex ID/URL, dblp URL/key, or title-like query. Returns normalized metadata and optionally citation/open-access enrichment.",
                "inputSchema": {
                    "type": "object",
                    "required": ["identifier"],
                    "properties": {
                        "identifier": { "type": "string" },
                        "include_citations": { "type": "boolean", "default": false },
                        "include_open_access": { "type": "boolean", "default": true }
                    }
                }
            },
            {
                "name": "academic_citations",
                "description": "Return a summary of citing and referenced papers for one academic identifier. Uses Semantic Scholar first and OpenAlex as fallback; this is an overview, not a full citation graph crawl.",
                "inputSchema": {
                    "type": "object",
                    "required": ["identifier"],
                    "properties": {
                        "identifier": { "type": "string" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 }
                    }
                }
            },
            {
                "name": "academic_read",
                "description": "Resolve and read an academic PDF as Markdown or text. Open-access sources are tried first: arXiv, Semantic Scholar, OpenAlex, Unpaywall. Sci-Hub is only considered when explicitly enabled in configuration and remains the last fallback.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "identifier": { "type": "string" },
                        "url": { "type": "string" },
                        "max_chars": { "type": "integer", "minimum": 1 },
                        "output_format": { "type": "string", "enum": ["markdown", "text"], "default": "markdown" }
                    }
                }
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_contains_existing_tools() {
        let names: Vec<_> = tools().into_iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            vec![
                "web_search",
                "get_sources",
                "web_fetch",
                "web_map",
                "doctor",
                "academic_search",
                "academic_get",
                "academic_citations",
                "academic_read"
            ]
        );
    }

    #[test]
    fn typed_web_search_args_accept_existing_shape() {
        let params: WebSearchParams = serde_json::from_value(json!({
            "query": "rust mcp",
            "platform": "x",
            "model": "grok-4",
            "extra_sources": 2,
            "recency_days": 7,
            "include_domains": ["example.com"],
            "exclude_domains": ["old.example"],
            "include_content": true,
            "response_format": "detailed"
        }))
        .expect("valid params");

        let input: WebSearchInput = params.into();
        assert_eq!(input.query, "rust mcp");
        assert_eq!(input.extra_sources, Some(2));
        assert_eq!(input.recency_days, Some(7));
        assert_eq!(input.include_domains, vec!["example.com"]);
    }

    #[test]
    fn typed_academic_args_accept_existing_shape() {
        let params: AcademicSearchParams = serde_json::from_value(json!({
            "query": "retrieval augmented generation",
            "sources": ["dblp", "arxiv"],
            "search_mode": "broad",
            "sort_by": "citations",
            "max_results": 5,
            "year_from": 2020,
            "year_to": 2026,
            "open_access_only": true,
            "include_abstract": true,
            "include_citations": false
        }))
        .expect("valid params");

        let input: AcademicSearchInput = params.into();
        assert_eq!(input.query, "retrieval augmented generation");
        assert_eq!(input.sources, vec!["dblp", "arxiv"]);
        assert_eq!(input.search_mode.as_deref(), Some("broad"));
        assert_eq!(input.sort_by.as_deref(), Some("citations"));
        assert_eq!(input.max_results, Some(5));
        assert_eq!(input.year_from, Some(2020));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_tool_returns_existing_web_map_shape() {
        let service = SearchService::fake_with_sources();
        let value = invoke_tool(
            &service,
            "web_map",
            json!({
                "url": "https://93.184.216.34",
                "max_results": 2
            }),
        )
        .await
        .expect("web_map should succeed");

        assert_eq!(value["url"], "https://93.184.216.34");
        assert_eq!(value["sources_count"], 2);
        assert_eq!(value["sources"].as_array().unwrap().len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_tool_rejects_web_map_out_of_range() {
        let service = SearchService::fake_with_sources();
        let err = invoke_tool(
            &service,
            "web_map",
            json!({
                "url": "https://93.184.216.34",
                "max_results": 51
            }),
        )
        .await
        .expect_err("max_results above 50 should fail");
        assert!(matches!(err, GrokSearchError::InvalidParams(_)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invoke_tool_doctor_accepts_verbose_param() {
        let service = SearchService::fake_with_sources();
        let value = invoke_tool(&service, "doctor", json!({ "verbose": true }))
            .await
            .expect("doctor should succeed");

        assert_eq!(value["diagnostics"]["debug_log"]["enabled"], false);
        assert!(value["diagnostics"]["limits"]["max_response_bytes"].is_number());
    }
}
