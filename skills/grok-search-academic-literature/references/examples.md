# Academic Literature Examples

Use academic tools for scholarly work instead of generic web search. For PDF
tools, provide exactly one locator: `identifier`, `url`, or `pdf_url`.

## Tool Choice

Use `academic_search` for discovery, `academic_get` for one known paper, and
`academic_citations` for a citation overview. Use the four intent-oriented PDF
tools directly:

- `academic_pdf_read`: readable PDF text only.
- `academic_pdf_structure`: LLM-assisted progressive paper structure.
- `academic_pdf_artifacts`: figures, tables, manifests, and visual completion.
- `academic_pdf_download`: raw PDF file only.

Do not use legacy `academic_read`, `academic_parse_pdf`, or
`academic_progressive_get` as first-choice agent tools. They exist for
compatibility and diagnostics. The new PDF tools share internal resolver,
download, cache, and parser code, so one public tool does not need to be called
before another.

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

Use `text_mode:"none"` only when debugging raw `pdf_oxide` extraction. Use
`include_processing:true` when comparing `none`, `light`, and `clean` modes or
when checking whether headers, captions, references, or table-like lines were
cleaned too aggressively. Use `include_raw_content:true` sparingly because it
can double the response size.

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
`view:"summary"` is the safest first call. Use the returned outline section IDs
for `view:"section"`; `section_id` is required for that view. Keep
`include_section_text:false` unless the user explicitly needs full section text;
the canonical cached structure still keeps section text internally.

The structure tool internally resolves, downloads, parses, checks cache, and
runs the LLM pass only when needed. Do not call a separate cache-key tool first.
Progressive metadata is heuristic and evidence-bound; title and abstract fields
may be absent when the PDF begins with publisher notices, copyright text, or
unusual front matter.

The progressive pass uses the configured LLM provider, currently the
Anthropic-compatible `minimax` or `anthropic` path. It chunks cleaned PDF text,
asks for bounded JSON signals, validates local patches/spans, and assembles the
section tree locally. It is not a free-form paper summarizer.

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

By default, `vision_profile` is `auto`. When an LLM key is configured, the tool
also runs the `artifact_micro` pass on selected high-risk pages. This passes
rendered page PNGs plus local page anchors and triage reasons to a
vision-capable Anthropic-compatible model. The model returns figure/table
completion candidates: normalized bboxes, caption bboxes, table headers/rows,
Markdown, status, confidence, and short notes. Local code validates/refines
those candidates before exposing them as `vision.figure_completions` and
`vision.table_completions`.

Save LLM-refined artifact diagnostics and crops:

```json
{
  "identifier": "1706.03762",
  "images_dir": "tmp/attention-images",
  "tables_dir": "tmp/attention-tables",
  "vision_profile": "auto",
  "vision_dir": "tmp/attention-vision",
  "vision_cache_policy": "auto"
}
```

Refined completions are the preferred artifact reading surface when
`vision_profile` is enabled, but they are still model-assisted candidates. Check
`validation.status`, `confidence`, `warnings`, and `raw_completions` for risky
results. The LLM artifact pass never edits body text and does not replace
deterministic artifacts; LLM outputs are marked as `llm_vision_refined_*`
artifacts when written.

Use `vision_profile:"off"` for deterministic pdf_oxide artifacts only. Use
`vision_cache_policy:"refresh"` after changing model, endpoint, prompt behavior,
or when checking whether a stale cached vision result hid a model improvement.
Set `extract_images:true` only with `images_dir`, and `extract_tables:true` only
with `tables_dir`; the tool rejects missing output directories for enabled
artifact types. Use `vision_dir` when debugging because it writes `vision.json`
with raw/refined/validation/report details. Use lower `vision_max_pages` for
quick checks and `vision_cache_policy:"refresh"` when comparing M3/M2.x or
prompt changes.

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

Existing files are rejected unless `overwrite` is true. Prefer
`academic_pdf_download` over `web_fetch` when the user specifically asks for the
PDF file. Prefer `academic_pdf_read` when the user asks for the paper content.

## Cache And Timing

Use `cache_policy:"refresh"` when validating a cold download or when upstream
PDF bytes may have changed. Use `cache_policy:"bypass"` when diagnosing cache
corruption or measuring network behavior. Normal reading should leave
`cache_policy` unset or `auto`.

PDF tool outputs can include `pdf_cache` diagnostics with `hit`, `stored`,
`bytes`, `attempts`, `download_elapsed_ms`, and warnings such as
`download_plan=...`, `download_strategy=...`, or individual `download_attempt`
records.

`cache_policy:"auto"` reads and writes the local PDF bytes cache. `refresh`
forces a fresh download and overwrites the cache. `bypass` skips the PDF cache.
`academic_pdf_structure` also uses the progressive structure cache; its
`cache_policy` controls whether the progressive result is reused or refreshed.

If `pdf_cache.warnings` reports that the cache path cannot be opened, set
`academic_pdf_cache_path` or `GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH` to a writable
directory. The PDF tools still work without cache, but repeated calls will pay
the full cold-download cost.

## Common Misuses

- Do not pass both `identifier` and `pdf_url`; exactly one locator is accepted.
- Do not call `academic_pdf_structure` just to get raw text; use
  `academic_pdf_read`.
- Do not call `academic_pdf_read` expecting image/table files; use
  `academic_pdf_artifacts`.
- Do not treat LLM figure/table completions as deterministic extraction; inspect
  validation and warnings when numbers or bboxes matter.
- Do not raise `max_chars` as a substitute for `view:"section"`; use the outline
  then request the needed section.
