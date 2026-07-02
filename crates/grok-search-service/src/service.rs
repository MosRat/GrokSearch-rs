use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::cache::SourceCache;
use crate::domain_filter::filter_sources_by_domains;
use crate::response_budget::apply_response_budget;
use grok_search_audit::{AuditRecorder, AuditStatus};
use grok_search_config::Config;
use grok_search_net::proxy::ProxyDiagnostics;
pub use grok_search_provider_core::{
    AcademicServiceProvider, AiProvider, SourceProvider, WechatProvider, ZhihuProvider,
};
use grok_search_source_core::{SourceCaps, SourceRouter};
use grok_search_types::model::search::{
    ContentBlock, SearchFilters, SearchMessage, SearchRequest, SearchResponse, SearchTool,
};
use grok_search_types::model::source::{merge_sources, Source};
use grok_search_types::model::tool::{GetSourcesOutput, WebSearchInput, WebSearchOutput};
use grok_search_types::{
    AcademicCitationsOutput, AcademicDownloadPdfOutput, AcademicGetOutput, AcademicParseOptions,
    AcademicParsePdfOutput, AcademicPdfArtifactsInput, AcademicPdfArtifactsOutput,
    AcademicPdfDownloadInput, AcademicPdfDownloadOutput, AcademicPdfReadInput,
    AcademicPdfReadOutput, AcademicPdfStructureInput, AcademicPdfStructureOutput,
    AcademicProgressiveGetInput, AcademicProgressiveGetOutput, AcademicReadOutput,
    AcademicSearchInput, AcademicSearchOutput, WechatSearchInput, WechatSearchOutput,
    ZhihuSearchInput, ZhihuSearchOutput,
};
use grok_search_types::{GrokSearchError, Result};

mod factory;
mod probe;
mod tools;
mod web;

#[cfg(test)]
pub(crate) use factory::resolve_default_model;
#[cfg(test)]
pub(crate) use factory::{FakeAiProvider, FakeSourceProvider};
pub(crate) use probe::Probe;

#[derive(Clone)]
pub struct SearchService {
    pub(crate) config: Config,
    pub(crate) ai: Arc<dyn AiProvider>,
    /// Model name written into every `SearchRequest` produced by the service.
    /// Resolved once from `config` at construction so each transport gets the
    /// model it actually understands: `grok_model` for Responses, and
    /// `openai_compatible_model` (falling back to `grok_model`) for the
    /// chat-completions transport. Per-call overrides via `WebSearchInput.model`
    /// still win.
    pub(crate) default_model: String,
    pub(crate) sources: Option<Arc<dyn SourceProvider>>,
    pub(crate) fallback_sources: Option<Arc<dyn SourceProvider>>,
    pub(crate) cache: Arc<Mutex<SourceCache>>,
    /// Shared reqwest client for the sources pipeline (same instance handed to
    /// providers). Stored here because resolve_content needs direct GET access.
    pub(crate) http_client: reqwest::Client,
    /// Specialist extractor router. Empty in Phase 1. Behind `Arc` so
    /// `SearchService: Clone` still holds (the router is not `Clone`).
    pub(crate) source_router: Arc<SourceRouter>,
    pub(crate) proxy_diagnostics: ProxyDiagnostics,
    pub(crate) academic: Option<Arc<dyn AcademicServiceProvider>>,
    pub(crate) wechat: Option<Arc<dyn WechatProvider>>,
    pub(crate) zhihu: Option<Arc<dyn ZhihuProvider>>,
    pub(crate) audit: AuditRecorder,
}

pub struct SearchServiceParts {
    pub config: Config,
    pub ai: Arc<dyn AiProvider>,
    pub sources: Option<Arc<dyn SourceProvider>>,
    pub fallback_sources: Option<Arc<dyn SourceProvider>>,
    pub http_client: reqwest::Client,
    pub source_router: SourceRouter,
    pub proxy_diagnostics: ProxyDiagnostics,
    pub academic: Option<Arc<dyn AcademicServiceProvider>>,
    pub wechat: Option<Arc<dyn WechatProvider>>,
    pub zhihu: Option<Arc<dyn ZhihuProvider>>,
}

impl SearchService {
    pub fn audit_snapshot(
        &self,
        query: grok_search_audit::AuditRecentQuery,
    ) -> grok_search_audit::AuditSnapshot {
        self.audit.snapshot(query)
    }

    pub fn audit_summary(&self) -> grok_search_audit::AuditSummary {
        self.audit.summary()
    }

    pub fn audit_recent(
        &self,
        query: grok_search_audit::AuditRecentQuery,
    ) -> Vec<grok_search_audit::AuditRecentCall> {
        self.audit.recent(query)
    }

    pub fn audit_clear(&self) -> Result<()> {
        self.audit.clear()
    }

    pub(crate) fn audit_result<T>(
        &self,
        request_id: &str,
        operation: &str,
        started_at_unix_ms: u128,
        start: Instant,
        result: &Result<T>,
        payload: serde_json::Value,
    ) {
        match result {
            Ok(_) => self.audit.record_tool_call(
                operation,
                request_id,
                started_at_unix_ms,
                start.elapsed(),
                AuditStatus::Success,
                None,
                payload,
            ),
            Err(err) => self.audit.record_tool_call(
                operation,
                request_id,
                started_at_unix_ms,
                start.elapsed(),
                AuditStatus::Error,
                Some(err.kind()),
                payload,
            ),
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
