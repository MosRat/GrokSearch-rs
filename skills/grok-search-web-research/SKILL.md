---
name: grok-search-web-research
description: Use GrokSearch-rs web research tools for open-web discovery, source follow-up, single-page reading, URL mapping, debugging search questions, news/topic research, and evidence gathering with web_search, get_sources, web_fetch, and web_map.
---

# GrokSearch Web Research

Use this skill when a task needs general web discovery, source follow-up, URL inventory, or page-level evidence through GrokSearch-rs.

## Workflow

1. Use `web_search` when the user has a topic, error message, question, product, news item, or vague lead rather than a known URL.
2. Use `response_format: "concise"` for broad discovery and `response_format: "detailed"` when inline source text matters.
3. Use `get_sources` with the returned `session_id` when you need to inspect more cached sources from the same search.
4. Use `web_fetch` when you already have an exact URL, need page details, or need stronger evidence than the synthesized answer.
5. Use `web_map` for URL discovery around a site or page, then fetch selected URLs with `web_fetch`.

Prefer exact tools over broad ones: do not use `web_search` for a URL the user already gave, and do not use `web_map` expecting page bodies. For scholarly paper PDFs, prefer the academic PDF tools; for GitHub or Hugging Face metadata, prefer `repo_metadata`.

Read `references/examples.md` for parameter patterns, example calls, and failure handling.
