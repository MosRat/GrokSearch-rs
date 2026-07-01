# Repo Intelligence Examples

## GitHub by URL

```json
{
  "url":"https://github.com/modelcontextprotocol/rust-sdk",
  "include_readme":true,
  "max_text_chars":12000
}
```

Use URL input when the user pasted a GitHub or Hugging Face link. It is the
least ambiguous form and lets the tool infer provider details.

## GitHub by Owner and Name

```json
{
  "provider":"github",
  "owner":"modelcontextprotocol",
  "name":"rust-sdk",
  "include_readme":false
}
```

## Hugging Face Model

```json
{
  "url":"https://huggingface.co/bert-base-uncased",
  "include_card":true,
  "max_text_chars":10000
}
```

## Hugging Face Dataset

```json
{
  "provider":"huggingface",
  "repo_id":"squad",
  "repo_type":"dataset",
  "include_card":true
}
```

## Boundaries

`repo_metadata` does not crawl commits, issues, files, releases, Spaces, or external links. Use `web_fetch` for a specific external page or repository page when deeper reading is required.

Use `include_readme` only for GitHub. Use `include_card` only for Hugging Face
models or datasets. Always set `max_text_chars` when the README/card is likely
large, because the metadata fields are usually enough for quick triage.

If the task is to compare papers or fetch arXiv/PDF metadata, use academic
tools. If the task is to inspect live docs, issue comments, release notes, or
an arbitrary file page, use `web_fetch` on the exact URL.
