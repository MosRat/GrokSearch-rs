# grok-search-rs

Run the GrokSearch-rs MCP server with npx:

```bash
npx grok-search-rs
```

See https://github.com/MosRat/GrokSearch-rs for configuration and MCP client examples.

Repo-local GrokSearch skills are included in the published package metadata and
can also be installed directly from the GitHub release asset:

```bash
curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash -s -- --target codex
```
