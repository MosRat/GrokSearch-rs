# Architecture

GrokSearch-rs is a Rust MCP server that keeps the original GrokSearch product boundary while making provider behavior explicit and testable.

```text
MCP client
  -> crates/grok-search-rs       CLI plus stdio / HTTP MCP entrypoints
      -> crates/grok-search-mcp  rmcp server adapter, shared handler, and transports
      -> crates/grok-search-runtime
          -> concrete runtime wiring from Config to providers, sources, academic service
      -> crates/grok-search-service
          -> crates/grok-search-provider-core
              -> shared AI/source/academic provider traits and capability errors
          -> crates/grok-search-source-core
              -> shared source extractor/router/caps abstractions
          -> crates/grok-search-types    shared request/response/source/error models
          -> source cache
      -> implementation crates used only by runtime
          -> crates/grok-search-auth     static API key or xAI OAuth token
          -> crates/grok-search-net      reqwest clients, proxy bootstrap, key rotation
          -> crates/grok-search-parse
              -> shared identifiers, title normalization, OpenAlex abstract, RRF/dedupe helpers
          -> crates/grok-search-content
              -> generic content parsing, truncation, and artifact file helpers
          -> crates/grok-search-pdf
              -> PDF byte guards, adaptive downloads, pdf_oxide parsing, pass-based text/image/table artifacts
          -> crates/grok-search-cache
              -> redb-backed PDF bytes cache and progressive reading cache
          -> crates/grok-search-llm
              -> Anthropic-compatible LLM client used by progressive PDF structure extraction
          -> crates/grok-search-providers
              -> Grok Responses provider: /v1/responses with web_search and optional x_search
              -> OpenAI-compatible chat-completions provider
              -> Tavily provider: search / extract / map
              -> Firecrawl provider: search / scrape fallback
          -> crates/grok-search-academic  CS literature metadata, citations, full-text PDF parsing
          -> crates/grok-search-sources  specialist fetch/render extractors
```

## Product Boundary

- `web_search` is the AI search path. Grok Responses is primary.
- `get_sources` retrieves cached sources by `session_id`.
- `web_fetch` fetches page content through Tavily Extract first, then Firecrawl scrape if configured.
- `web_map` discovers URLs through Tavily Map.
- Tavily and Firecrawl are not the default answer generators inside `web_search`; they provide enrichment, fallback sources, fetch, and map capability.
- Agents should use `web_search` for concise sourced summaries, call `get_sources` before source-specific claims, citation lists, or follow-up fetches, and call `web_fetch` for exact page evidence, quotes, technical details, or when the summary is insufficient.
- Agents should use `academic_search` / `academic_get` / `academic_citations` for computer-science paper discovery, metadata, and citation summaries. For PDFs, prefer the intent-oriented tools: `academic_pdf_read` for text, `academic_pdf_structure` for LLM-assisted progressive reading structure, `academic_pdf_artifacts` for images/tables/manifests, and `academic_pdf_download` for saving the raw PDF.

## Academic Layer

`grok-search-academic` owns scholarly orchestration and the concrete academic providers. Shared scholarly mechanics that are useful outside the academic service live below it: identifiers, title normalization, OpenAlex abstract reconstruction, and RRF/dedupe are in `grok-search-parse`; generic content parsing, truncation, and artifact file helpers are in `grok-search-content`; PDF byte validation, adaptive downloads, `pdf_oxide` parsing, and image/table artifact extraction are in `grok-search-pdf`; the `AcademicProvider` trait and capability defaults are in `grok-search-provider-core`.

Academic providers are capability-based: dblp and Crossref are metadata-first, Semantic Scholar and OpenAlex add citations/references, arXiv and open-access locations provide PDFs, Unpaywall resolves DOI-based OA full text, and Sci-Hub is disabled by default and only used as a final explicitly configured fallback. Results are normalized into `AcademicPaper` while provenance URLs remain regular `Source` values.

Academic PDF tools are facades over shared internals rather than stages users
must chain manually. Each of `academic_pdf_read`, `academic_pdf_structure`,
`academic_pdf_artifacts`, and `academic_pdf_download` accepts exactly one
locator (`identifier`, `url`, or `pdf_url`) and internally performs resolve,
download, cache lookup, parsing, and optional LLM structure extraction as
needed. Legacy `academic_read`, `academic_parse_pdf`,
`academic_download_pdf`, and `academic_progressive_get` remain as compatibility
or diagnostic entry points, but they are not the default tools exposed to
agents.

The deterministic PDF parser remains synchronous and model-free in
`grok-search-pdf`. Its pipeline runs validation, raw page extraction, text
signal analysis, cleanup (`none`, `light`, or `clean`), image/table artifact
extraction, artifact refinement, optional writes, and final truncation. The LLM
progressive pass lives above it in `grok-search-academic`: it consumes the
parsed source bundle, calls `grok-search-llm`, validates chunk JSON and local
patches, assembles `AcademicProgressivePaper`, and stores the canonical result
in the progressive cache.

PDF downloads use an internal adaptive downloader. By default the academic
facade reads/writes a redb PDF bytes cache, applies retry backoff, records
elapsed time, and chooses between full-body and HTTP range strategies based on
the source host. Known large/flaky PDF hosts such as arXiv, CVF Open Access,
and ACL Anthology try range downloads first; other hosts keep full-body
downloads first and fall back to range/direct strategies when needed.

## Provider Layer

Provider traits are defined in `grok-search-provider-core`. `grok-search-providers` implements the web-side providers (Grok/OpenAI-compatible/Tavily/Firecrawl), while `grok-search-academic` implements academic providers. `grok-search-runtime` is the only crate that wires concrete implementations into the service; `grok-search-service` depends on abstractions and shared types only.

The service builds an internal search request and sends one Responses payload:

| Provider | Endpoint | Tool shape |
|---|---|---|
| Grok Responses | `{GROK_SEARCH_URL normalized to /v1}/responses` | `{"type":"web_search"}` plus optional `{"type":"x_search"}` |

The provider returns normalized assistant content and normalized `Source` values. Empty content or missing native sources are treated as unverifiable for `web_search`.

Authentication is separated from the Responses provider:

- `api_key` mode returns the configured `GROK_SEARCH_API_KEY` as a static Bearer token.
- `oauth` mode reads the local auth file, refreshes the access token when it is near expiry, and returns the fresh Bearer token for the same `/v1/responses` request body.

OAuth login is not a service boundary. `grok-search-rs login` temporarily listens on `127.0.0.1:56121` for the browser callback, stores the token file, then exits. Normal local MCP operation remains stdio by default; Streamable HTTP MCP is an explicit server mode.

## Source Provenance

Source extraction has the same split: `grok-search-source-core` owns the router/extractor trait and no-match sentinel, while `grok-search-sources` only implements specialist renderers for GitHub, StackExchange, arXiv, and Wikipedia and provides a config-to-router factory.

Sources retain their origin through the `provider` field:

- `grok_responses`: native Responses citation or web search source.
- `tavily_enrichment`: supplemental Tavily source after Grok succeeds.
- `tavily_fallback`: Tavily source used because Grok failed or was unverifiable.
- `firecrawl_enrichment`: Firecrawl source used when Tavily supplemental or fallback source lookup returns nothing.
- `tavily` / `firecrawl`: direct provider source before orchestration rewrites provenance.

## Fallback Rules

`web_search` falls back to source providers when:

- the Grok Responses request fails,
- the provider response content is empty,
- the provider response has no verifiable native sources.

Fallback tries Tavily first, then Firecrawl when configured. The output exposes `search_provider`, `fallback_used`, and `fallback_reason` so MCP clients can distinguish a native Grok result from fallback-source handling.

## MCP Transport

The binary exposes the same MCP handler over two transports:

- stdio, the default for local agent integrations and `grok-search-rs mcp`.
- Streamable HTTP, started explicitly with `grok-search-rs mcp-http`.

Both transports handle:

- `initialize`
- `tools/list`
- `tools/call`

The HTTP transport uses `rmcp`'s official Streamable HTTP service with local
sessions, SSE keepalive, Host/Origin validation, request body limits, and
graceful shutdown. It binds to `127.0.0.1:8787` by default. Non-loopback binds
are rejected unless `mcp_http_auth_token` / `GROK_SEARCH_MCP_HTTP_AUTH_TOKEN`
is set, and Bearer tokens are never logged.

Tool responses are returned as structured MCP content and also serialized JSON
inside text content for broad client compatibility.
