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

## `get_sources`

Inspect cached sources from a previous `web_search`:

```json
{"session_id":"<session_id>","offset":0,"limit":5}
```

Use the returned `next_offset` for pagination. This does not issue a new search.

## `web_fetch`

Fetch one known URL:

```json
{"url":"https://github.com/modelcontextprotocol/specification","max_chars":12000}
```

Prefer `web_fetch` over `web_search` for exact URLs, quotes, issue/PR details, StackOverflow answers, arXiv pages, and Wikipedia pages.

## `web_map`

Discover URLs around a site or page:

```json
{"url":"https://example.com/docs","max_results":20}
```

Use mapped URLs with `web_fetch` when content details are needed.

## Failure Handling

- If `web_search` is truncated, use `web_fetch` on a source URL or `get_sources` with pagination.
- If `web_fetch` returns generic extraction, treat the text as best-effort and cite the URL carefully.
- If a URL is blocked by URL policy, do not retry with private or local network variants.
