# grok-search-rs

PyPI wrapper for the native `grok-search-rs` MCP server.

```bash
uv tool install grok-search-rs
uvx grok-search-rs --version
```

The wheel contains the native Rust binary for your platform and exposes the
same `grok-search-rs` command.

The release also includes repo-local GrokSearch skills. Install them into an
agent skill directory with:

```bash
curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash -s -- --target codex
```
