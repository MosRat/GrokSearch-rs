use std::time::Instant;

use serde_json::json;

use super::*;

impl SearchService {
    pub async fn academic_search(
        &self,
        input: AcademicSearchInput,
    ) -> Result<AcademicSearchOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async { self.academic_service()?.search(input).await }.await;
        self.log_result(
            &request_id,
            "academic_search",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn wechat_search(&self, mut input: WechatSearchInput) -> Result<WechatSearchOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let query_chars = input.query.chars().count();
        let result = if input.query.trim().is_empty() {
            Err(GrokSearchError::InvalidParams(
                "wechat_search.query is required".to_string(),
            ))
        } else {
            input.max_results = Some(input.max_results.unwrap_or(10));
            input.pages = Some(input.pages.unwrap_or(1));
            input.max_content_chars = input
                .max_content_chars
                .or(self.config.fetch_max_chars)
                .or(Some(self.config.enrich_max_chars));
            self.wechat_provider()?.search(input).await
        };
        self.log_result(
            &request_id,
            "wechat_search",
            started_at_unix_ms,
            start,
            &result,
            json!({ "query_chars": query_chars }),
        );
        result
    }

    pub async fn zhihu_search(&self, mut input: ZhihuSearchInput) -> Result<ZhihuSearchOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let query_chars = input.query.chars().count();
        let result = if input.query.trim().is_empty() {
            Err(GrokSearchError::InvalidParams(
                "zhihu_search.query is required".to_string(),
            ))
        } else {
            input.count = Some(input.count.unwrap_or(10));
            self.zhihu_provider()?.search(input).await
        };
        self.log_result(
            &request_id,
            "zhihu_search",
            started_at_unix_ms,
            start,
            &result,
            json!({ "query_chars": query_chars }),
        );
        result
    }

    pub async fn academic_get(
        &self,
        identifier: &str,
        include_citations: bool,
        include_open_access: bool,
        extract_material_links: bool,
    ) -> Result<AcademicGetOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async {
            self.academic_service()?
                .get(
                    identifier,
                    include_citations,
                    include_open_access,
                    extract_material_links,
                )
                .await
        }
        .await;
        self.log_result(
            &request_id,
            "academic_get",
            started_at_unix_ms,
            start,
            &result,
            json!({ "identifier": identifier }),
        );
        result
    }

    pub async fn academic_citations(
        &self,
        identifier: &str,
        limit: Option<usize>,
    ) -> Result<AcademicCitationsOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async {
            self.academic_service()?
                .citations(identifier, limit.unwrap_or(10))
                .await
        }
        .await;
        self.log_result(
            &request_id,
            "academic_citations",
            started_at_unix_ms,
            start,
            &result,
            json!({ "identifier": identifier, "limit": limit }),
        );
        result
    }

    pub async fn academic_read(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicReadOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async {
            self.academic_service()?
                .read(identifier, url, max_chars, output_format, parse_options)
                .await
        }
        .await;
        self.log_result(
            &request_id,
            "academic_read",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_parse_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        max_chars: Option<usize>,
        output_format: Option<String>,
        parse_options: Option<AcademicParseOptions>,
    ) -> Result<AcademicParsePdfOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async {
            self.academic_service()?
                .parse_pdf(identifier, url, max_chars, output_format, parse_options)
                .await
        }
        .await;
        self.log_result(
            &request_id,
            "academic_parse_pdf",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_download_pdf(
        &self,
        identifier: Option<String>,
        url: Option<String>,
        output_path: String,
        overwrite: bool,
    ) -> Result<AcademicDownloadPdfOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async {
            self.academic_service()?
                .download_pdf(identifier, url, output_path, overwrite)
                .await
        }
        .await;
        self.log_result(
            &request_id,
            "academic_download_pdf",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_read(
        &self,
        input: AcademicPdfReadInput,
    ) -> Result<AcademicPdfReadOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async { self.academic_service()?.pdf_read(input).await }.await;
        self.log_result(
            &request_id,
            "academic_pdf_read",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_structure(
        &self,
        input: AcademicPdfStructureInput,
    ) -> Result<AcademicPdfStructureOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async { self.academic_service()?.pdf_structure(input).await }.await;
        self.log_result(
            &request_id,
            "academic_pdf_structure",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_artifacts(
        &self,
        input: AcademicPdfArtifactsInput,
    ) -> Result<AcademicPdfArtifactsOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async { self.academic_service()?.pdf_artifacts(input).await }.await;
        self.log_result(
            &request_id,
            "academic_pdf_artifacts",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_pdf_download(
        &self,
        input: AcademicPdfDownloadInput,
    ) -> Result<AcademicPdfDownloadOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let result = async { self.academic_service()?.pdf_download(input).await }.await;
        self.log_result(
            &request_id,
            "academic_pdf_download",
            started_at_unix_ms,
            start,
            &result,
            json!({}),
        );
        result
    }

    pub async fn academic_progressive_get(
        &self,
        input: AcademicProgressiveGetInput,
    ) -> Result<AcademicProgressiveGetOutput> {
        let request_id = self.audit.request_id();
        let started_at_unix_ms = grok_search_audit::now_unix_ms();
        let start = Instant::now();
        let cache_key = input.cache_key.clone();
        let result = async { self.academic_service()?.progressive_get(input).await }.await;
        self.log_result(
            &request_id,
            "academic_progressive_get",
            started_at_unix_ms,
            start,
            &result,
            json!({ "cache_key": cache_key }),
        );
        result
    }

    pub fn warm_academic_institutional_access(&self) {
        if let Some(academic) = &self.academic {
            academic.warm_institutional_access();
        }
    }

    fn academic_service(&self) -> Result<&dyn AcademicServiceProvider> {
        self.academic
            .as_ref()
            .map(|service| service.as_ref())
            .ok_or(GrokSearchError::MissingConfig(
                "GROK_SEARCH_ACADEMIC_ENABLED",
            ))
    }

    fn wechat_provider(&self) -> Result<&dyn WechatProvider> {
        self.wechat
            .as_ref()
            .map(|provider| provider.as_ref())
            .ok_or(GrokSearchError::MissingConfig("wechat provider"))
    }

    fn zhihu_provider(&self) -> Result<&dyn ZhihuProvider> {
        self.zhihu
            .as_ref()
            .map(|provider| provider.as_ref())
            .ok_or(GrokSearchError::MissingConfig(
                "ZHIHU_ACCESS_SECRET or ZHIHU_API_KEY",
            ))
    }

    fn log_result<T>(
        &self,
        request_id: &str,
        operation: &str,
        started_at_unix_ms: u128,
        start: Instant,
        result: &Result<T>,
        payload: serde_json::Value,
    ) {
        self.audit_result(
            request_id,
            operation,
            started_at_unix_ms,
            start,
            result,
            payload,
        );
    }
}
