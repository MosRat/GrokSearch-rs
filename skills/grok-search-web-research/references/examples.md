# Web Research Examples

## `web_search`

Broad discovery:

```json
{"query":"Rust MCP server structured tool schema best practices","response_format":"concise"}
```

Recent or domain-scoped discovery:

```json
{
  "query":"OpenAI API Responses model migration",
  "recency_days":30,
  "include_domains":["platform.openai.com"],
  "response_format":"detailed"
}
```

Use `exclude_domains` for noisy sources and `extra_sources` when the default source count is too small.

Use `response_format:"concise"` when you mostly need source discovery or a
short answer. Use `response_format:"detailed"` when inline source text is
important and the response budget can handle it. `response_format` takes
precedence over `include_content`.

`include_domains` and `exclude_domains` constrain supplemental search sources;
they are not a replacement for fetching an exact known URL. For official docs
or policy-sensitive answers, combine domain constraints with follow-up
`web_fetch` on the strongest sources.

## `get_sources`

Inspect cached sources from a previous `web_search`:

```json
{"session_id":"<session_id>","offset":0,"limit":5}
```

Use the returned `next_offset` for pagination. This does not issue a new search.
Use `get_sources` when a `web_search` result was truncated or when you want to
inspect source metadata/text already fetched for the same session. It is not a
fresh search and cannot discover newer or different pages.

## `web_fetch`

Fetch one known URL:

```json
{"url":"https://github.com/modelcontextprotocol/specification","max_chars":12000}
```

Prefer `web_fetch` over `web_search` for exact URLs, quotes, issue/PR details, StackOverflow answers, arXiv pages, and Wikipedia pages.
`web_fetch` returns one page with `original_length` and `truncated` metadata.
If truncated, raise `max_chars` or fetch a narrower source. For paper PDFs,
prefer `academic_pdf_read` or `academic_pdf_download`; `web_fetch` is for web
pages and known URL evidence.

## `web_map`

Discover URLs around a site or page:

```json
{"url":"https://example.com/docs","max_results":20}
```

Use mapped URLs with `web_fetch` when content details are needed.
`web_map` is URL inventory, not content reading. It is useful before a focused
fetch pass on docs sites, sitemap-like pages, or product documentation.

## Failure Handling

- If `web_search` is truncated, use `web_fetch` on a source URL or `get_sources` with pagination.
- If `web_fetch` returns generic extraction, treat the text as best-effort and cite the URL carefully.
- If a URL is blocked by URL policy, do not retry with private or local network variants.
- If search results are too broad, narrow with domains, recency, or a more
  specific query before increasing `extra_sources`.
- If the task is academic discovery, Chinese WeChat/Zhihu discovery, or repo
  metadata, prefer the domain-specific tools before falling back to web search.
