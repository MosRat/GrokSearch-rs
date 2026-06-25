# Architecture

GrokSearch-rs is a Rust MCP server that keeps the original GrokSearch product boundary while making provider behavior explicit and testable.

```text
MCP client
  -> crates/grok-search-rs       CLI and stdio entrypoint
      -> crates/grok-search-mcp  rmcp server adapter and tool schemas
      -> crates/grok-search-service
          -> crates/grok-search-auth     static API key or xAI OAuth token
          -> crates/grok-search-net      reqwest clients, proxy bootstrap, key rotation
          -> crates/grok-search-provider-core
              -> shared AI/source/academic provider traits and capability errors
          -> crates/grok-search-source-core
              -> shared source extractor/router/caps abstractions
          -> crates/grok-search-parse
              -> shared identifiers, title normalization, OpenAlex abstract, RRF/dedupe helpers
          -> crates/grok-search-content
              -> shared PDF byte guards, pdf_oxide parsing, truncation helpers
          -> crates/grok-search-providers
              -> Grok Responses provider: /v1/responses with web_search and optional x_search
              -> OpenAI-compatible chat-completions provider
              -> Tavily provider: search / extract / map
              -> Firecrawl provider: search / scrape fallback
          -> crates/grok-search-academic  CS literature metadata, citations, full-text PDF parsing
          -> crates/grok-search-sources  specialist fetch/render extractors
          -> crates/grok-search-types    shared request/response/source/error models
          -> source cache
```

## Product Boundary

- `web_search` is the AI search path. Grok Responses is primary.
- `get_sources` retrieves cached sources by `session_id`.
- `web_fetch` fetches page content through Tavily Extract first, then Firecrawl scrape if configured.
- `web_map` discovers URLs through Tavily Map.
- Tavily and Firecrawl are not the default answer generators inside `web_search`; they provide enrichment, fallback sources, fetch, and map capability.
- Agents should use `web_search` for concise sourced summaries, call `get_sources` before source-specific claims, citation lists, or follow-up fetches, and call `web_fetch` for exact page evidence, quotes, technical details, or when the summary is insufficient.
- Agents should use `academic_search` / `academic_get` / `academic_citations` / `academic_read` for computer-science paper discovery, metadata, citation summaries, and PDF text extraction rather than forcing scholarly tasks through generic web tools.

## Academic Layer

`grok-search-academic` owns scholarly orchestration and the concrete academic providers. Shared scholarly mechanics that are useful outside the academic service live below it: identifiers, title normalization, OpenAlex abstract reconstruction, and RRF/dedupe are in `grok-search-parse`; PDF byte validation, `pdf_oxide` parsing, and truncation are in `grok-search-content`; the `AcademicProvider` trait and capability defaults are in `grok-search-provider-core`.

Academic providers are capability-based: dblp and Crossref are metadata-first, Semantic Scholar and OpenAlex add citations/references, arXiv and open-access locations provide PDFs, Unpaywall resolves DOI-based OA full text, and Sci-Hub is disabled by default and only used as a final explicitly configured fallback. Results are normalized into `AcademicPaper` while provenance URLs remain regular `Source` values.

## Provider Layer

Provider traits are defined in `grok-search-provider-core`. `grok-search-providers` implements the web-side providers (Grok/OpenAI-compatible/Tavily/Firecrawl), while `grok-search-academic` implements academic providers. `grok-search-service` depends on the traits and concrete crates only at the orchestration boundary.

The service builds an internal search request and sends one Responses payload:

| Provider | Endpoint | Tool shape |
|---|---|---|
| Grok Responses | `{GROK_SEARCH_URL normalized to /v1}/responses` | `{"type":"web_search"}` plus optional `{"type":"x_search"}` |

The provider returns normalized assistant content and normalized `Source` values. Empty content or missing native sources are treated as unverifiable for `web_search`.

Authentication is separated from the Responses provider:

- `api_key` mode returns the configured `GROK_SEARCH_API_KEY` as a static Bearer token.
- `oauth` mode reads the local auth file, refreshes the access token when it is near expiry, and returns the fresh Bearer token for the same `/v1/responses` request body.

OAuth login is not a service boundary. `grok-search-rs login` temporarily listens on `127.0.0.1:56121` for the browser callback, stores the token file, then exits. Normal MCP operation remains stdio only.

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

The binary is a stdio JSON-RPC server. It handles:

- `initialize`
- `tools/list`
- `tools/call`

Tool responses are serialized JSON inside MCP text content for broad client compatibility.
