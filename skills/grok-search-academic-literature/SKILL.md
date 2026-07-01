---
name: grok-search-academic-literature
description: Use GrokSearch-rs academic tools for computer-science paper discovery, metadata lookup, citations, PDF text reading, LLM progressive PDF structure, PDF image/table artifacts, and PDF download workflows with academic_search, academic_get, academic_citations, academic_pdf_read, academic_pdf_structure, academic_pdf_artifacts, and academic_pdf_download.
---

# GrokSearch Academic Literature

Use this skill for computer-science literature work rather than forcing scholarly tasks through generic web search or legacy PDF tools.

## Workflow

1. Use `academic_search` for topic discovery and ranked paper lists.
2. Use `academic_get` for one known DOI, arXiv ID, Semantic Scholar ID, OpenAlex ID, dblp key, URL, or title-like identifier.
3. Use `academic_citations` for citing/referenced paper summaries.
4. Use `academic_pdf_read` when the user wants readable PDF text. It defaults to cleaned text and does not require any prior PDF tool call.
5. Use `academic_pdf_structure` when the user wants section-level paper understanding. It resolves, downloads, parses, caches, and runs the LLM progressive pass internally.
6. Use `academic_pdf_artifacts` when the user wants figures, tables, manifests, or LLM-refined figure/table completion. It does not return body text or progressive section summaries.
7. Use `academic_pdf_download` only when the user needs the raw PDF file saved locally.

Use exactly one PDF locator: `identifier`, `url`, or `pdf_url`. Do not call a cache-key lookup tool before the main PDF tools; cache is internal. Keep `cache_policy:"auto"` unless debugging stale bytes or timing cold downloads. `academic_pdf_artifacts` defaults `vision_profile` to `auto`: it enables LLM-refined artifact completion when an LLM key is configured and otherwise stays deterministic. Use `vision_profile:"off"` for a deterministic-only comparison. Legacy `academic_read`, `academic_parse_pdf`, and `academic_download_pdf` are compatibility paths, not preferred agent-facing tools.

Read `references/examples.md` for source selection, PDF profiles, cache policy, and example calls.
