---
name: grok-search-social-search
description: Use GrokSearch-rs social/content search tools for WeChat public-account articles and Zhihu site content, including source-account filtering, article body fetching, Zhihu OpenAPI metadata lookup, and Chinese research/content discovery with wechat_search and zhihu_search.
---

# GrokSearch Social Search

Use this skill for Chinese content discovery on WeChat public accounts and Zhihu.

## Workflow

1. Use `wechat_search` for WeChat public-account articles, especially known accounts such as 机器之心, Datawhale, or 量子位.
2. Use `account` only for exact local filtering against the parsed source name.
3. Keep `pages` small for quick recall; increase it when account filtering leaves too few results.
4. Use `include_content: false` for metadata-only scans and `include_content: true` when article bodies are needed.
5. Use `zhihu_search` for Zhihu OpenAPI site search; it returns normalized metadata and does not fetch full page content.

Read `references/examples.md` for query patterns, quality checks, and example calls.
