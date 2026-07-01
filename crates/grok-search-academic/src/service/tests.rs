use super::*;
use crate::providers::{
    parse_dblp_search, parse_openalex_work, parse_semantic_paper,
    without_openalex_reference_sources,
};
use grok_search_types::AcademicPdfStructureProfile;
use serde_json::json;

#[test]
fn identifier_normalizes_common_academic_ids() {
    assert_eq!(
        parse_identifier("https://arxiv.org/pdf/1706.03762.pdf"),
        Identifier::Arxiv("1706.03762".to_string())
    );
    assert_eq!(
        parse_identifier("10.48550/arXiv.1706.03762"),
        Identifier::Arxiv("1706.03762".to_string())
    );
    assert_eq!(
        parse_identifier("10.1145/3368089.3409742"),
        Identifier::Doi("10.1145/3368089.3409742".to_string())
    );
    assert_eq!(
        parse_identifier("https://openalex.org/W2741809807"),
        Identifier::OpenAlex("https://openalex.org/W2741809807".to_string())
    );
}

#[test]
fn dblp_fixture_parses_core_metadata() {
    let value = json!({
        "result": { "hits": { "hit": [{
            "info": {
                "title": "Attention Is All You Need",
                "authors": { "author": [{ "text": "Ashish Vaswani" }, { "text": "Noam Shazeer" }] },
                "year": "2017",
                "venue": "NIPS",
                "doi": "10.5555/3295222.3295349",
                "url": "https://dblp.org/rec/conf/nips/VaswaniSPUJGKP17"
            }
        }] } }
    });
    let papers = parse_dblp_search(&value);
    assert_eq!(papers.len(), 1);
    assert_eq!(papers[0].title, "Attention Is All You Need");
    assert_eq!(papers[0].authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
    assert_eq!(papers[0].doi.as_deref(), Some("10.5555/3295222.3295349"));
    assert_eq!(papers[0].sources[0].provider.as_ref(), "dblp");
}

#[test]
fn semantic_fixture_parses_ids_and_counts() {
    let value = json!({
        "paperId": "abc123",
        "title": "A Paper",
        "authors": [{ "name": "Ada Lovelace" }],
        "year": 2024,
        "venue": "SOSP",
        "abstract": "Abstract",
        "url": "https://semanticscholar.org/paper/abc123",
        "externalIds": { "DOI": "10.1/example", "ArXiv": "2401.00001" },
        "citationCount": 7,
        "referenceCount": 3,
        "openAccessPdf": { "url": "https://example.com/paper.pdf" }
    });
    let paper = parse_semantic_paper(&value);
    assert_eq!(paper.semantic_scholar_id.as_deref(), Some("abc123"));
    assert_eq!(paper.arxiv_id.as_deref(), Some("2401.00001"));
    assert_eq!(paper.citation_count, Some(7));
    assert_eq!(paper.open_access, Some(true));
}

#[test]
fn openalex_inverted_abstract_is_reconstructed() {
    let value = json!({
        "id": "https://openalex.org/W1",
        "title": "Open Work",
        "publication_year": 2025,
        "authorships": [{ "author": { "display_name": "Grace Hopper" } }],
        "abstract_inverted_index": { "hello": [0], "world": [1] },
        "cited_by_count": 42,
        "referenced_works": ["https://openalex.org/W0"],
        "open_access": { "is_oa": true },
        "best_oa_location": { "pdf_url": "https://example.com/oa.pdf", "license": "cc-by" }
    });
    let paper = parse_openalex_work(&value);
    assert_eq!(paper.abstract_text.as_deref(), Some("hello world"));
    assert_eq!(paper.citation_count, Some(42));
    assert_eq!(paper.reference_count, Some(1));
    assert_eq!(paper.pdf_url.as_deref(), Some("https://example.com/oa.pdf"));
    assert!(paper
        .sources
        .iter()
        .any(|source| source.provider.as_ref() == "openalex_reference"));
    let search_paper = without_openalex_reference_sources(paper);
    assert!(!search_paper
        .sources
        .iter()
        .any(|source| source.provider.as_ref() == "openalex_reference"));
}

#[test]
fn rrf_merge_dedupes_by_doi_and_keeps_sources() {
    let a = AcademicPaper {
        id: "a".into(),
        title: "Same".into(),
        doi: Some("10.1/same".into()),
        sources: vec![Source::new("https://dblp.org/x", "dblp")],
        ..Default::default()
    };
    let b = AcademicPaper {
        id: "b".into(),
        title: "Same".into(),
        doi: Some("10.1/same".into()),
        citation_count: Some(10),
        sources: vec![Source::new("https://semanticscholar.org/x", "semantic")],
        ..Default::default()
    };
    let merged = rrf_merge(vec![("dblp".into(), vec![a]), ("semantic".into(), vec![b])]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].citation_count, Some(10));
    assert_eq!(merged[0].sources.len(), 2);
}

#[tokio::test]
async fn academic_search_zero_max_results_returns_empty_without_providers() {
    let service = AcademicService::new(
        reqwest::Client::new(),
        Config::from_env_map(Vec::<(String, String)>::new()),
    );
    let output = service
        .search(AcademicSearchInput {
            query: "transformer".into(),
            max_results: Some(0),
            ..Default::default()
        })
        .await
        .expect("zero max_results should be valid");
    assert_eq!(output.papers_count, 0);
    assert!(output.papers.is_empty());
    assert!(output.sources_used.is_empty());
}

#[test]
fn title_like_get_rejects_dblp_near_miss() {
    let id = Identifier::Query("Attention Is All You Need".into());
    let near_miss = AcademicPaper {
        title: "Attentional Transfer is All You Need: Technology-aware Layout Pattern Generation."
            .into(),
        ..Default::default()
    };
    let exact = AcademicPaper {
        title: "Attention Is All You Need".into(),
        ..Default::default()
    };
    assert!(!resolved_paper_matches_identifier(&id, &near_miss));
    assert!(resolved_paper_matches_identifier(&id, &exact));
}

#[test]
fn nonsense_query_filters_unrelated_papers() {
    let paper = AcademicPaper {
        title: "Spectroscopic Needs for Calibration of LSST Photometric Redshifts".into(),
        abstract_text: Some("Dark energy survey calibration".into()),
        ..Default::default()
    };
    assert!(!search_result_is_relevant(
        "zzzxxy nonexistent paper qwertyuiopasdf",
        &paper
    ));
    assert!(search_result_is_relevant(
        "photometric redshifts calibration",
        &paper
    ));
}

#[test]
fn academic_search_modes_select_expected_default_sources() {
    assert_eq!(
        selected_sources(&[], AcademicSearchMode::Balanced).expect("default sources"),
        vec!["dblp", "semantic", "arxiv"]
    );
    assert_eq!(
        selected_sources(&[], AcademicSearchMode::Precise).expect("default sources"),
        vec!["dblp", "semantic", "arxiv"]
    );
    assert_eq!(
        selected_sources(&[], AcademicSearchMode::Broad).expect("default sources"),
        vec!["dblp", "semantic", "arxiv", "openalex", "crossref"]
    );
}

#[test]
fn selected_sources_accept_common_semantic_scholar_aliases() {
    assert_eq!(
        selected_sources(
            &["semantic_scholar".into(), "semanticscholar,s2".into()],
            AcademicSearchMode::Balanced
        )
        .expect("aliases should normalize"),
        vec!["semantic", "semantic", "semantic"]
    );
}

#[test]
fn selected_sources_reject_unknown_values() {
    let err = selected_sources(
        &["semantic_scholar".into(), "scholar".into()],
        AcademicSearchMode::Balanced,
    )
    .expect_err("unknown source should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[test]
fn citation_identifiers_prefer_provider_native_ids() {
    let paper = AcademicPaper {
        title: "Attention Is All You Need".into(),
        doi: Some("10.48550/arXiv.1706.03762".into()),
        arxiv_id: Some("1706.03762".into()),
        semantic_scholar_id: Some("semantic-id".into()),
        openalex_id: Some("https://openalex.org/W123".into()),
        ..Default::default()
    };
    assert_eq!(
        citation_identifiers_for_paper(&paper),
        vec![
            Identifier::Semantic("semantic-id".into()),
            Identifier::OpenAlex("https://openalex.org/W123".into()),
            Identifier::Doi("10.48550/arXiv.1706.03762".into()),
            Identifier::Arxiv("1706.03762".into()),
        ]
    );
}

#[test]
fn academic_search_mode_rejects_unknown_values() {
    let err = search_mode(Some("exploratory")).expect_err("unknown mode should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[test]
fn academic_sort_by_rejects_unknown_values() {
    assert_eq!(
        academic_sort_by(Some("citations")).expect("valid sort"),
        AcademicSortBy::Citations
    );
    let err = academic_sort_by(Some("impact")).expect_err("unknown sort should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[test]
fn precise_relevance_requires_title_overlap() {
    let abstract_only = AcademicPaper {
        title: "Generic Systems Paper".into(),
        abstract_text: Some("large language model evaluation".into()),
        ..Default::default()
    };
    let title_match = AcademicPaper {
        title: "A Survey on Evaluation of Large Language Models".into(),
        ..Default::default()
    };
    assert!(!precise_search_result_is_relevant(
        "large language model evaluation",
        &abstract_only
    ));
    assert!(precise_search_result_is_relevant(
        "large language model evaluation",
        &title_match
    ));
}

#[test]
fn strong_overlap_rejects_sort_boosted_partial_matches() {
    let partial = AcademicPaper {
        title: "A comprehensive survey of loss functions and metrics in deep learning".into(),
        abstract_text: Some("survey methods for deep learning".into()),
        ..Default::default()
    };
    let relevant = AcademicPaper {
        title: "Retrieval-Augmented Generation for Large Language Models: A Survey".into(),
        ..Default::default()
    };
    assert!(!search_result_has_strong_overlap(
        "retrieval augmented generation survey",
        &partial
    ));
    assert!(search_result_has_strong_overlap(
        "retrieval augmented generation survey",
        &relevant
    ));
}

#[test]
fn multi_token_relevance_rejects_single_generic_token_match() {
    let generic = AcademicPaper {
        title: "R: A Language and Environment for Statistical Computing".into(),
        abstract_text: Some("A statistical programming environment".into()),
        ..Default::default()
    };
    let relevant = AcademicPaper {
        title: "A Survey on Evaluation of Large Language Models".into(),
        abstract_text: Some("Evaluation methods for large language model systems".into()),
        ..Default::default()
    };
    assert!(!search_result_is_relevant(
        "large language model evaluation",
        &generic
    ));
    assert!(search_result_is_relevant(
        "large language model evaluation",
        &relevant
    ));
}

#[test]
fn academic_ranking_prioritizes_exact_title_then_overlap() {
    let mut papers = vec![
        AcademicPaper {
            title: "Large Models in General".into(),
            abstract_text: Some("large language model evaluation".into()),
            citation_count: Some(10_000),
            ..Default::default()
        },
        AcademicPaper {
            title: "A Survey on Evaluation of Large Language Models".into(),
            citation_count: Some(10),
            ..Default::default()
        },
    ];
    rank_academic_results(
        "A Survey on Evaluation of Large Language Models",
        AcademicSortBy::Relevance,
        &mut papers,
    );
    assert_eq!(
        papers[0].title,
        "A Survey on Evaluation of Large Language Models"
    );
}

#[test]
fn citation_sort_boosts_cited_relevant_papers_without_beating_exact_title() {
    let mut papers = vec![
        AcademicPaper {
            title: "Large Language Model Evaluation Notes".into(),
            citation_count: Some(5),
            ..Default::default()
        },
        AcademicPaper {
            title: "Large Language Model Evaluation Survey".into(),
            citation_count: Some(5_000),
            ..Default::default()
        },
        AcademicPaper {
            title: "Large Language Model Evaluation".into(),
            citation_count: Some(1),
            ..Default::default()
        },
    ];
    rank_academic_results(
        "Large Language Model Evaluation",
        AcademicSortBy::Citations,
        &mut papers,
    );
    assert_eq!(papers[0].title, "Large Language Model Evaluation");
    assert_eq!(papers[1].title, "Large Language Model Evaluation Survey");
}

#[test]
fn year_filter_keeps_unknown_years_and_bounds_known_years() {
    let unknown = AcademicPaper {
        title: "Unknown".into(),
        year: None,
        ..Default::default()
    };
    let old = AcademicPaper {
        title: "Old".into(),
        year: Some(2023),
        ..Default::default()
    };
    let current = AcademicPaper {
        title: "Current".into(),
        year: Some(2024),
        ..Default::default()
    };
    let future = AcademicPaper {
        title: "Future".into(),
        year: Some(2025),
        ..Default::default()
    };
    assert!(paper_matches_year_filter(&unknown, Some(2024), Some(2024)));
    assert!(!paper_matches_year_filter(&old, Some(2024), Some(2024)));
    assert!(paper_matches_year_filter(&current, Some(2024), Some(2024)));
    assert!(!paper_matches_year_filter(&future, Some(2024), Some(2024)));
}

#[test]
fn title_query_fallback_selector_requires_exact_normalized_title() {
    let exact = AcademicPaper {
        title: "Attention Is All You Need".into(),
        ..Default::default()
    };
    let near_miss = AcademicPaper {
        title: "Attention Is Almost All You Need".into(),
        ..Default::default()
    };
    let found = select_best_title_match(
        "attention is all you need",
        vec![near_miss.clone(), exact.clone()],
    )
    .expect("exact title");
    assert_eq!(found.title, exact.title);
    assert!(select_best_title_match("attention is all you need", vec![near_miss]).is_none());
}

#[test]
fn title_query_selector_prefers_canonical_scholarly_metadata() {
    let query = "Canonical Systems Paper";
    let low_confidence = AcademicPaper {
        title: query.into(),
        year: Some(2025),
        doi: Some("10.65215/example".into()),
        sources: vec![
            Source::new("https://openalex.org/W1", "openalex"),
            Source::new("https://doi.org/10.65215/example", "crossref"),
        ],
        ..Default::default()
    };
    let canonical = AcademicPaper {
        title: query.into(),
        authors: vec!["Ada Lovelace".into(), "Grace Hopper".into()],
        year: Some(2017),
        venue: Some("Conference on Systems".into()),
        arxiv_id: Some("1701.00001".into()),
        semantic_scholar_id: Some("semantic-paper".into()),
        citation_count: Some(10_000),
        sources: vec![
            Source::new(
                "https://semanticscholar.org/paper/semantic-paper",
                "semantic",
            ),
            Source::new("https://arxiv.org/abs/1701.00001", "arxiv"),
        ],
        ..Default::default()
    };
    let found = select_best_title_match(query, vec![low_confidence, canonical.clone()])
        .expect("canonical match");
    assert_eq!(found.semantic_scholar_id, canonical.semantic_scholar_id);
}

#[test]
fn title_query_selector_rejects_near_title_even_when_highly_cited() {
    let near = AcademicPaper {
        title: "Canonical Systems Paper Extended".into(),
        citation_count: Some(100_000),
        semantic_scholar_id: Some("near".into()),
        sources: vec![Source::new(
            "https://semanticscholar.org/paper/near",
            "semantic",
        )],
        ..Default::default()
    };
    assert!(select_best_title_match("Canonical Systems Paper", vec![near]).is_none());
}

#[test]
fn title_query_selector_allows_low_confidence_provider_when_only_exact_candidate() {
    let query = "Niche Exact Paper";
    let crossref_only = AcademicPaper {
        title: query.into(),
        year: Some(2024),
        doi: Some("10.1234/niche".into()),
        sources: vec![Source::new("https://doi.org/10.1234/niche", "crossref")],
        ..Default::default()
    };
    let found = select_best_title_match(query, vec![crossref_only.clone()])
        .expect("single exact candidate should still be usable");
    assert_eq!(found.doi, crossref_only.doi);
}

#[test]
fn pdf_locator_validation_requires_one_location() {
    assert!(ensure_valid_locator(
        &AcademicPdfLocator {
            pdf_url: Some("https://example.com/paper.pdf".to_string()),
            ..Default::default()
        },
        "academic_pdf_read"
    )
    .is_ok());

    let err = ensure_valid_locator(&AcademicPdfLocator::default(), "academic_pdf_read")
        .expect_err("missing locator should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));

    let err = ensure_valid_locator(
        &AcademicPdfLocator {
            identifier: Some("arXiv:1706.03762".to_string()),
            pdf_url: Some("https://example.com/paper.pdf".to_string()),
            ..Default::default()
        },
        "academic_pdf_read",
    )
    .expect_err("multiple locators should fail");
    assert!(matches!(err, GrokSearchError::InvalidParams(_)));
}

#[test]
fn pdf_structure_profiles_map_to_llm_options() {
    let config = Config::from_env_map([
        ("GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL", "MiniMax-M3"),
        ("GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE", "strict"),
    ]);

    let strict = llm_options_for_structure(
        &AcademicPdfStructureInput {
            locator: AcademicPdfLocator {
                pdf_url: Some("https://example.com/paper.pdf".to_string()),
                ..Default::default()
            },
            cache_policy: Some(AcademicPdfCachePolicy::Refresh),
            ..Default::default()
        },
        &config,
    );
    assert_eq!(strict.enabled, Some(true));
    assert_eq!(strict.model.as_deref(), Some("MiniMax-M3"));
    assert_eq!(strict.max_chunk_chars, Some(5_500));
    assert_eq!(strict.overlap_chars, Some(700));
    assert_eq!(strict.concurrency, Some(1));
    assert_eq!(strict.max_output_tokens, Some(1_800));
    assert_eq!(strict.cache_enabled, Some(true));
    assert_eq!(strict.cache_refresh, Some(true));

    let fast_bypass = llm_options_for_structure(
        &AcademicPdfStructureInput {
            locator: AcademicPdfLocator {
                pdf_url: Some("https://example.com/paper.pdf".to_string()),
                ..Default::default()
            },
            profile: Some(AcademicPdfStructureProfile::Fast),
            cache_policy: Some(AcademicPdfCachePolicy::Bypass),
            model: Some("custom-model".to_string()),
            ..Default::default()
        },
        &config,
    );
    assert_eq!(fast_bypass.model.as_deref(), Some("custom-model"));
    assert_eq!(fast_bypass.max_chunk_chars, Some(4_500));
    assert_eq!(fast_bypass.cache_enabled, Some(false));
    assert_eq!(fast_bypass.cache_refresh, Some(false));
}

#[test]
fn pdf_cache_key_uses_hash_without_raw_url() {
    let location = FullTextLocation {
        url: "https://example.com/paper.pdf?token=secret".to_string(),
        source: "direct_url".to_string(),
        status: "direct_url".to_string(),
    };
    let key = pdf_cache_key(&location, 1024);
    assert!(key.starts_with("academic_pdf:v1:"));
    assert!(!key.contains("token=secret"));
    assert!(!key.contains("paper.pdf"));
}

#[test]
fn pdf_download_retry_policy_covers_timeout_and_5xx() {
    assert_eq!(pdf_download_retry_delay_ms(1), 600);
    assert_eq!(pdf_download_retry_delay_ms(2), 1_200);
    assert!(is_retryable_pdf_download_error(&GrokSearchError::Timeout(
        "slow".into()
    )));
    assert!(is_retryable_pdf_download_error(&GrokSearchError::Upstream(
        "academic pdf returned HTTP 503".into()
    )));
    assert!(!is_retryable_pdf_download_error(
        &GrokSearchError::InvalidParams("bad".into())
    ));
}

#[test]
fn canonical_merge_starts_from_best_candidate() {
    let weak = AcademicPaper {
        title: "Same Paper".into(),
        doi: Some("10.65215/weak".into()),
        sources: vec![Source::new("https://doi.org/10.65215/weak", "crossref")],
        ..Default::default()
    };
    let strong = AcademicPaper {
        title: "Same Paper".into(),
        semantic_scholar_id: Some("sem".into()),
        arxiv_id: Some("2401.00001".into()),
        citation_count: Some(500),
        sources: vec![Source::new(
            "https://semanticscholar.org/paper/sem",
            "semantic",
        )],
        ..Default::default()
    };
    let merged = merge_canonical_candidates(vec![weak, strong]);
    assert_eq!(merged.semantic_scholar_id.as_deref(), Some("sem"));
    assert_eq!(merged.arxiv_id.as_deref(), Some("2401.00001"));
}

#[test]
fn citation_summary_cleanup_removes_openalex_reference_sources() {
    let relation = AcademicPaper {
        title: "Related".into(),
        sources: vec![
            Source::new("https://openalex.org/W0", "openalex"),
            Source::new("https://openalex.org/W1", "openalex_reference"),
        ],
        ..Default::default()
    };
    let cleaned = clean_citation_summary(AcademicCitationSummary {
        citations: vec![relation.clone()],
        references: vec![relation],
    });
    assert!(cleaned.citations[0]
        .sources
        .iter()
        .all(|source| source.provider.as_ref() != "openalex_reference"));
    assert!(cleaned.references[0]
        .sources
        .iter()
        .all(|source| source.provider.as_ref() != "openalex_reference"));
}

#[tokio::test]
async fn academic_read_download_timeout_returns_tool_error_promptly() {
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration as StdDuration;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let url = format!("http://{}/slow.pdf", listener.local_addr().unwrap());
    let _handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        thread::sleep(StdDuration::from_millis(500));
    });

    let mut config = Config::from_env_map([
        ("GROK_SEARCH_TIMEOUT_SECONDS", "60"),
        ("GROK_SEARCH_ACADEMIC_PDF_CACHE_ENABLED", "false"),
    ]);
    config.timeout = std::time::Duration::from_millis(50);
    let service = AcademicService::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client"),
        config,
    );
    let err = service
        .read(None, Some(url), Some(10), Some("text".to_string()), None)
        .await
        .expect_err("download should time out");
    assert!(
        matches!(err, GrokSearchError::Timeout(_)),
        "expected timeout, got {err:?}"
    );
}

#[tokio::test]
async fn pdf_download_uses_cache_on_second_call() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let requests_for_thread = Arc::clone(&requests);
    let _handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        requests_for_thread.fetch_add(1, Ordering::SeqCst);
        let mut buf = [0u8; 512];
        let _ = stream.read(&mut buf);
        let body = b"%PDF-cache-test";
        let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(body);
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let config = Config::from_env_map([
        (
            "GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH",
            dir.path()
                .join("pdf-cache.redb")
                .to_string_lossy()
                .to_string(),
        ),
        ("GROK_SEARCH_TIMEOUT_SECONDS", "5".to_string()),
    ]);
    let service = AcademicService::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client"),
        config,
    );
    let location = FullTextLocation {
        url,
        source: "direct_url".to_string(),
        status: "direct_url".to_string(),
    };

    let first = service
        .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
        .await
        .expect("first download");
    let second = service
        .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
        .await
        .expect("second download");

    assert!(!first.cache.hit);
    assert!(first.cache.stored);
    assert!(second.cache.hit);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pdf_download_refresh_bypasses_existing_cache() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let url = format!("http://{}/paper.pdf", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let requests_for_thread = Arc::clone(&requests);
    let _handle = thread::spawn(move || {
        for idx in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept request");
            requests_for_thread.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0u8; 512];
            let _ = stream.read(&mut buf);
            let body = if idx == 0 {
                b"%PDF-cache-first".as_slice()
            } else {
                b"%PDF-cache-refresh".as_slice()
            };
            let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        }
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let config = Config::from_env_map([
        (
            "GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH",
            dir.path()
                .join("pdf-cache.redb")
                .to_string_lossy()
                .to_string(),
        ),
        ("GROK_SEARCH_TIMEOUT_SECONDS", "5".to_string()),
    ]);
    let service = AcademicService::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client"),
        config,
    );
    let location = FullTextLocation {
        url,
        source: "direct_url".to_string(),
        status: "direct_url".to_string(),
    };

    let first = service
        .download_pdf_for_location(&location, AcademicPdfCachePolicy::Auto)
        .await
        .expect("first download");
    let refreshed = service
        .download_pdf_for_location(&location, AcademicPdfCachePolicy::Refresh)
        .await
        .expect("refresh download");

    assert_eq!(first.bytes, b"%PDF-cache-first");
    assert_eq!(refreshed.bytes, b"%PDF-cache-refresh");
    assert!(!refreshed.cache.hit);
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn academic_read_parse_timeout_is_mapped_to_timeout_error() {
    let err = match parse_pdf_bytes_with_timeout(
        b"%PDF-1.7\n".to_vec(),
        "text".to_string(),
        10,
        None,
        std::time::Duration::from_secs(0),
        "https://example.com/paper.pdf",
    )
    .await
    {
        Ok(_) => panic!("parse should time out before the blocking task completes"),
        Err(err) => err,
    };
    assert!(
        matches!(err, GrokSearchError::Timeout(_)),
        "expected timeout, got {err:?}"
    );
}

#[tokio::test]
async fn academic_read_rejects_invalid_output_format_before_fetching() {
    let service = AcademicService::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client"),
        Config::from_env_map(Vec::<(String, String)>::new()),
    );
    let err = service
        .read(
            None,
            Some("http://127.0.0.1:1/paper.pdf".to_string()),
            Some(10),
            Some("html".to_string()),
            None,
        )
        .await
        .expect_err("invalid format should fail before network fetch");
    assert!(
        matches!(err, GrokSearchError::InvalidParams(_)),
        "expected invalid params, got {err:?}"
    );
}

#[test]
fn material_links_detect_common_research_artifacts() {
    let text = "Code: https://github.com/example/repo Dataset https://huggingface.co/datasets/org/data Demo https://huggingface.co/spaces/org/demo";
    let links = material_links_from_text(text, "abstract");
    let kinds: Vec<_> = links.iter().map(|link| link.kind.as_str()).collect();
    assert_eq!(kinds, vec!["code", "dataset", "demo"]);
    assert!(links.iter().all(|link| link.confidence == "high"));
}

#[tokio::test]
async fn academic_read_fulltext_locations_include_deduped_fallback_candidates() {
    let service = AcademicService::new(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test client"),
        Config::from_env_map(Vec::<(String, String)>::new()),
    );
    let paper = AcademicPaper {
        title: "Paper".into(),
        arxiv_id: Some("2401.00001".into()),
        pdf_url: Some("https://arxiv.org/pdf/2401.00001".into()),
        ..Default::default()
    };
    let locations = service
        .resolve_fulltext_locations(&paper)
        .await
        .expect("locations");
    assert_eq!(
        locations
            .iter()
            .filter(|location| location.url == "https://arxiv.org/pdf/2401.00001")
            .count(),
        1
    );
    assert!(locations
        .iter()
        .any(|location| location.source == "paper" || location.source == "arxiv"));
}
