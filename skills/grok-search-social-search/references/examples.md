# Social Search Examples

## `wechat_search`

Search WeChat articles with body extraction:

```json
{
  "query":"大模型 Agent 评测",
  "max_results":10,
  "pages":2,
  "include_content":true,
  "max_content_chars":8000
}
```

Filter by public account:

```json
{
  "query":"多模态 大模型",
  "account":"机器之心",
  "max_results":5,
  "pages":5,
  "include_content":true
}
```

The upstream query is ordinary keyword recall. Do not rely on Boolean operators, `source:`, `公众号:`, or other field syntax. Run multiple calls and merge results when OR/NOT logic is needed.

Use `pages` to widen recall before account filtering. Use `max_results` for the
final number of returned articles. `account` is an exact comparison against the
parsed article source, so try the visible public-account name exactly as it
appears in results.

Check each article's `quality`:

- `source_match`: whether local account filtering matched.
- `url_resolved`: whether the real `mp.weixin.qq.com` URL was resolved.
- `content_fetched`: whether body extraction succeeded.
- `warnings`: per-article quality issues.

If `url_resolved` or `content_fetched` is false, treat the item as metadata
only. Sogou/WeChat pages can change markup, redirect, block, or require
verification; failures should not invalidate other results in the same call.

## `zhihu_search`

Search Zhihu metadata:

```json
{"query":"如何理解 rave 文化","count":5}
```

Use `count` from 1 to 10. The tool requires `zhihu_api_key` or a compatible environment variable configured in GrokSearch-rs.
`zhihu_search` calls the Zhihu OpenAPI and returns normalized metadata:
`title`, `url`, `author_name`, `summary`, `vote_up_count`, `comment_count`, and
`edit_time`. It does not fetch the full Zhihu page body. Use `web_fetch` on a
returned URL only when page-level reading is needed and allowed by URL policy.

## Choosing Between Tools

- Use `wechat_search` when the source is an official account or a known Chinese technical media account.
- Use `zhihu_search` for Q&A/discussion-style discovery and author metadata.
- Use `web_search` if the task needs broader web coverage beyond WeChat/Zhihu.
- Use multiple `wechat_search` calls for OR/NOT workflows, then merge/filter
  locally; do not encode those operators in the upstream query.
