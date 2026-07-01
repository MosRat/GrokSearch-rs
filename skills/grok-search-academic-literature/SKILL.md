---
name: grok-search-academic-literature
description: Use GrokSearch-rs academic tools for computer-science paper discovery, metadata lookup, citations, PDF text reading, LLM progressive PDF structure, PDF image/table artifacts, and PDF download workflows with academic_search, academic_get, academic_citations, academic_pdf_read, academic_pdf_structure, academic_pdf_artifacts, and academic_pdf_download.
---

# GrokSearch Academic Literature

Use this skill for computer-science literature work rather than forcing scholarly tasks through generic web search.

## Workflow

1. Use `academic_search` for topic discovery and ranked paper lists.
2. Use `academic_get` for one known DOI, arXiv ID, Semantic Scholar ID, OpenAlex ID, dblp key, URL, or title-like identifier.
3. Use `academic_citations` for citing/referenced paper summaries.
4. Use `academic_pdf_read` for PDF text or Markdown. It is independent and does not require another PDF tool first.
5. Use `academic_pdf_structure` for progressive paper structure, section summaries, and cached LLM-assisted reading views.
6. Use `academic_pdf_artifacts` for image/table extraction and manifests without returning body text.
7. Use `academic_pdf_download` only when the user needs the raw PDF saved locally.

Use exactly one PDF locator: `identifier`, `url`, or `pdf_url`. Keep `cache_policy:"auto"` unless debugging stale bytes or timing cold downloads. Legacy `academic_read`, `academic_parse_pdf`, and `academic_download_pdf` are compatibility paths, not the preferred agent-facing tools.

Read `references/examples.md` for source selection, PDF profiles, cache policy, and example calls.
