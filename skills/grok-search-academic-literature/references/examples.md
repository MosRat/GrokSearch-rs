# Academic Literature Examples

## `academic_search`

Balanced topic search:

```json
{
  "query":"retrieval augmented generation evaluation benchmark",
  "search_mode":"balanced",
  "max_results":10,
  "include_abstract":true
}
```

Broad recent search:

```json
{
  "query":"long context transformer retrieval",
  "search_mode":"broad",
  "sort_by":"date",
  "year_from":2023,
  "max_results":20,
  "extract_material_links":true
}
```

Use `sources` only when the user asks to constrain providers, for example `["arxiv","semantic"]`.

## `academic_get`

Resolve a known paper:

```json
{"identifier":"1706.03762","include_citations":true,"include_open_access":true}
```

Use this before PDF reading when you need normalized metadata or open-access links.

## `academic_citations`

Citation overview:

```json
{"identifier":"10.48550/arXiv.1706.03762","limit":20}
```

This returns an overview, not a full citation graph crawl.

## `academic_read`

Read a paper as Markdown:

```json
{
  "identifier":"https://arxiv.org/abs/1706.03762",
  "max_chars":30000,
  "output_format":"markdown"
}
```

Use `url` instead of `identifier` when the user provides a direct PDF URL.

## `academic_parse_pdf`

Parse and save Markdown:

```json
{
  "identifier":"1706.03762",
  "output_format":"markdown",
  "parse_options":{
    "save_markdown_path":"tmp/attention.md",
    "extract_material_links":true
  }
}
```

Image/table extraction options may report unsupported capabilities depending on the parser.

## `academic_download_pdf`

Download without parsing:

```json
{
  "identifier":"1706.03762",
  "output_path":"tmp/attention.pdf",
  "overwrite":false
}
```

Existing files are rejected unless `overwrite` is true.
