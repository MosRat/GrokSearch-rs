# Testing

## Local Verification

Run the full local verification suite:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

For a release-style local pass, use:

```bash
scripts/check-workspace.sh
```

## Targeted Tests

Each crate owns the tests for its boundary. Prefer the narrowest command while
iterating, then run the workspace suite before committing.

| Layer | Command | Covers |
|---|---|
| Shared types | `cargo test -p grok-search-types` | `Source` merge behavior, tool output serialization |
| Config | `cargo test -p grok-search-config` | env/TOML precedence, paths, redacted diagnostics |
| Auth | `cargo test -p grok-search-auth` | OAuth token store and credential provider behavior |
| Net | `cargo test -p grok-search-net` | proxy discovery/bootstrap helpers and key rotation |
| Providers | `cargo test -p grok-search-providers` | Grok/OpenAI-compatible adapters, Tavily/Firecrawl parsing and key failover |
| Sources | `cargo test -p grok-search-sources` | GitHub, StackExchange, arXiv, Wikipedia rendering |
| Service | `cargo test -p grok-search-service` | orchestration, fallback, enrichment, budgets, doctor |
| MCP | `cargo test -p grok-search-mcp` | tool list/schema compatibility and typed argument parsing |
| Binary | `cargo test -p grok-search-rs` | CLI crate compilation and binary test harness |

Useful single-test shortcuts:

```bash
cargo test -p grok-search-config --test config
cargo test -p grok-search-providers --test adapter_grok_responses
cargo test -p grok-search-service --test service_contract
cargo test -p grok-search-sources --test sources_render
```

## Live Smoke Testing

Live provider tests require real API keys and should not be committed as logs.

Recommended smoke matrix:

1. `GROK_SEARCH_URL=https://api.x.ai` or another compatible gateway root URL.
2. `GROK_SEARCH_X_SEARCH=false` for baseline Responses `web_search` only.
3. `GROK_SEARCH_X_SEARCH=true` only when the gateway is known to preserve `x_search`.
4. Tavily fallback by forcing an empty or source-less Grok response in tests.
5. `web_fetch` against a stable public URL, first with Tavily, then with Firecrawl fallback.
6. `web_map` with a small `max_results` value.

Store live logs under `logs/`; the directory is ignored by git.
