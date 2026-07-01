---
name: grok-search-repo-intelligence
description: Use GrokSearch-rs repository metadata tooling for GitHub repositories and Hugging Face models or datasets, including README/card retrieval, provider-specific identifiers, and lightweight repo intelligence with repo_metadata.
---

# GrokSearch Repo Intelligence

Use this skill when a task needs structured GitHub or Hugging Face repository metadata without crawling the full repo.

## Workflow

1. Use `repo_metadata` with `url` when the user provides a GitHub or Hugging Face URL.
2. Use `provider` plus `repo_id` when no URL is available.
3. Use `owner` plus `name` for GitHub-style inputs.
4. Set `include_readme` for GitHub README text.
5. Set `include_card` for Hugging Face model or dataset cards.
6. Set `max_text_chars` when a README/card may be large.

Use this as lightweight repository intelligence, not a repository crawler. It does not inspect commits, issues, releases, file trees, Spaces runtime state, or external links. Use `web_fetch` for a specific repository page when deeper page reading is needed.

Read `references/examples.md` for supported input shapes and examples.
