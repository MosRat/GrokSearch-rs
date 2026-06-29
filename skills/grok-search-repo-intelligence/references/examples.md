# Repo Intelligence Examples

## GitHub by URL

```json
{
  "url":"https://github.com/modelcontextprotocol/rust-sdk",
  "include_readme":true,
  "max_text_chars":12000
}
```

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
