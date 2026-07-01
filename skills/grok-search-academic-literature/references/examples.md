# Academic Literature Examples

Use academic tools for scholarly work instead of generic web search. For PDF
tools, provide exactly one locator: `identifier`, `url`, or `pdf_url`.

## Discovery

Balanced topic search:

```json
{
  "query": "retrieval augmented generation evaluation benchmark",
  "search_mode": "balanced",
  "max_results": 10,
  "include_abstract": true
}
```

Broad recent search:

```json
{
  "query": "long context transformer retrieval",
  "search_mode": "broad",
  "sort_by": "date",
  "year_from": 2023,
  "max_results": 20,
  "extract_material_links": true
}
```

Use `sources` only when the user asks to constrain providers, for example
`["arxiv","semantic"]`. The canonical Semantic Scholar source name is
`semantic`; `semantic_scholar`, `semanticscholar`, and `s2` are accepted
compatibility aliases.

## Metadata

Resolve a known paper:

```json
{
  "identifier": "1706.03762",
  "include_citations": true,
  "include_open_access": true,
  "extract_material_links": true
}
```

Citation overview:

```json
{"identifier": "10.48550/arXiv.1706.03762", "limit": 20}
```

`academic_get` and `academic_citations` accept arXiv IDs, arXiv URLs, and arXiv
DOIs such as `10.48550/arXiv.1706.03762`. `academic_citations` internally
resolves to provider-native Semantic Scholar/OpenAlex identifiers when
available, then returns an overview, not a full citation graph crawl.

## PDF Text

Read a paper as processed Markdown:

```json
{
  "identifier": "https://arxiv.org/abs/1706.03762",
  "text_mode": "clean",
  "max_chars": 30000,
  "cache_policy": "auto"
}
```

Use `pdf_url` when the user gives a direct PDF URL:

```json
{
  "pdf_url": "https://arxiv.org/pdf/1706.03762",
  "text_mode": "clean",
  "include_processing": true
}
```

Inspect raw extraction alongside processed output:

```json
{
  "identifier": "1706.03762",
  "text_mode": "clean",
  "include_raw_content": true,
  "include_processing": true,
  "max_chars": 50000
}
```

Use `text_mode:"none"` only when debugging raw `pdf_oxide` extraction.

## Progressive Structure

Get a compact LLM-assisted structure:

```json
{
  "identifier": "1706.03762",
  "view": "summary",
  "profile": "balanced",
  "cache_policy": "auto"
}
```

Fetch the full structure and save the canonical JSON:

```json
{
  "identifier": "1706.03762",
  "view": "full",
  "profile": "strict",
  "include_section_text": false,
  "save_json_path": "tmp/attention.progressive.json"
}
```

Read one section after inspecting the outline:

```json
{
  "identifier": "1706.03762",
  "view": "section",
  "section_id": "sec-003",
  "include_section_text": true
}
```

Prefer `profile:"fast"` for a cheap overview, `balanced` for normal use, and
`strict` when section boundaries or JSON repair quality matter more than speed.
The structure tool internally resolves, downloads, parses, checks cache, and
runs the LLM pass only when needed. Do not call a separate cache-key tool first.
Progressive metadata is heuristic and evidence-bound; title and abstract fields
may be absent when the PDF begins with publisher notices, copyright text, or
unusual front matter.

## Artifacts

Extract images and tables without returning body text:

```json
{
  "identifier": "1706.03762",
  "extract_images": true,
  "images_dir": "tmp/attention-images",
  "extract_tables": true,
  "tables_dir": "tmp/attention-tables",
  "cache_policy": "auto"
}
```

Image extraction exports filtered bitmap XObjects as PNG files and writes
`images.json`. It does not reconstruct vector-only figures or perform OCR.
Table extraction writes `tables.json` and Markdown snippets for detected
tables; layout-heavy or sparse tables can be filtered or missed.

## Download

Download without parsing:

```json
{
  "identifier": "1706.03762",
  "output_path": "tmp/attention.pdf",
  "overwrite": false,
  "cache_policy": "auto"
}
```

Existing files are rejected unless `overwrite` is true.

## Cache And Timing

Use `cache_policy:"refresh"` when validating a cold download or when upstream
PDF bytes may have changed. Use `cache_policy:"bypass"` when diagnosing cache
corruption or measuring network behavior. Normal reading should leave
`cache_policy` unset or `auto`.

PDF tool outputs can include `pdf_cache` diagnostics with `hit`, `stored`,
`bytes`, `attempts`, `download_elapsed_ms`, and warnings such as
`download_plan=...`, `download_strategy=...`, or individual `download_attempt`
records.

If `pdf_cache.warnings` reports that the cache path cannot be opened, set
`academic_pdf_cache_path` or `GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH` to a writable
directory. The PDF tools still work without cache, but repeated calls will pay
the full cold-download cost.
