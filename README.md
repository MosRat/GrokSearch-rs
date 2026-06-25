# GrokSearch-rs

![GrokSearch-rs product banner](assets/groksearch-rs-banner.png)

**A lightweight Rust MCP server for Grok / OpenAI‑compatible web search, plus Tavily fetch/map and Firecrawl fallback.**

`grok-search-rs` is an **MCP stdio server** — your client (Claude Code, Codex, Cursor, VS Code, …) launches it; you do not run it directly. It exposes one set of tools (`web_search`, `get_sources`, `web_fetch`, `web_map`, `doctor`) and supports two upstream transports so you can plug into either xAI's official API or any OpenAI‑compatible relay.

---

## Features

- 🔎 **Live web search** with cited sources, cached for follow‑up `get_sources` calls. Opt‑in `include_content` enriches the top sources with full extracted text in one call.
- 📏 **Response budgeting** — `web_search` keeps responses inside agent context limits: only the top `max_inline_sources` carry inline text, a whole‑response char budget (`response_max_chars`, default 60k) trims tail sources with recovery notes, `response_format: "concise" | "detailed"` picks the payload size, and `get_sources` pages through cached sources with `offset`/`limit`. The session cache always keeps full content.
- 🧩 **Structured `web_fetch`** — GitHub issues/PRs, StackExchange/MathOverflow, arXiv, and Wikipedia URLs are parsed by specialist extractors into clean Markdown (title, state/labels, accepted‑answer ordering, abstracts, vote‑sorted answers). Anything else falls back to the generic Tavily → Firecrawl chain. Output carries `source_type` and a `fallback_reason` when a specialist was skipped.
- 🔀 **Two transports** — native xAI Responses (`/v1/responses`) **or** any OpenAI‑compatible chat‑completions gateway (`/v1/chat/completions`). Pick by env vars; no flag.
- 🔐 **Optional Grok OAuth mode** — `login/status/logout` commands store a local xAI OAuth token for Responses auth, so the MCP server can run without `GROK_SEARCH_API_KEY`.
- 📥 **Tavily fetch / map** for full‑text extraction and link discovery, with **Firecrawl** as automatic fallback. `TAVILY_API_KEY` and `FIRECRAWL_API_KEY` accept comma‑separated key lists — keys rotate round‑robin with automatic failover on rate/quota errors.
- 🐦 **Optional X/Twitter search** via `x_search` (Responses transport only).
- 🩺 **`doctor`** — connectivity probe + redacted config in one tool call.
- 🗂 **Single global config file** so multiple MCP clients share one set of keys.

---

## Install

```bash
npm install -g grok-search-rs
# or
uv tool install grok-search-rs
# or run without installing
uvx grok-search-rs --version
```

The npm and PyPI packages ship a native Rust binary; the `grok-search-rs` command is what your MCP client launches.

---

## Quick Start

1. After `npm install -g grok-search-rs` or `uv tool install grok-search-rs`, add this MCP server entry to your client config:

   ```json
   {
     "grok-search-rs": {
       "command": "grok-search-rs",
       "args": [],
       "env": {
         "GROK_SEARCH_API_KEY": "",
         "GROK_SEARCH_URL": "",
         "GROK_SEARCH_MODEL": "grok-4.20-fast",
         "TAVILY_API_KEY": "",
         "TAVILY_API_URL": "https://api.tavily.com",
         "FIRECRAWL_API_KEY": ""
       }
     }
   }
   ```

   For Codex TOML config:

   ```toml
   [mcp_servers.grok-search-rs]
   type = "stdio"
   command = "grok-search-rs"

   [mcp_servers.grok-search-rs.env]
   FIRECRAWL_API_KEY = ""
   GROK_SEARCH_API_KEY = ""
   GROK_SEARCH_MODEL = "grok-4.20-fast"
   GROK_SEARCH_URL = ""
   TAVILY_API_KEY = ""
   TAVILY_API_URL = "https://api.tavily.com"
   ```

   Put your real keys in the empty values. If your client expects a top-level `mcpServers` / `mcp_servers` object, place the `grok-search-rs` entry under that section.

2. Optional: scaffold a shared global config file instead of duplicating env blocks in every MCP client:

   ```bash
   grok-search-rs --init
   $EDITOR ~/.config/grok-search-rs/config.toml
   ```

3. Verify:

   ```text
   Ask your assistant: "call doctor"
   ```

   Successful output shows `reachable: true` for each enabled upstream and `transport: Responses` (or `ChatCompletions`).

---

## Configuration

Pick **one** transport group. Both Tavily and Firecrawl keys are shared across transports.

### A. Native Grok Responses (default)

| Variable | Default | Purpose |
|---|---|---|
| `GROK_SEARCH_AUTH_MODE` | `api_key` | `api_key` uses `GROK_SEARCH_API_KEY`; `oauth` uses the local token from `grok-search-rs login`. |
| `GROK_SEARCH_API_KEY` | — *(required in `api_key` mode)* | Bearer token for the Grok / xAI gateway. |
| `GROK_SEARCH_AUTH_FILE` | `<home>/.config/grok-search-rs/auth.json` | Optional OAuth token file override. |
| `GROK_SEARCH_URL` | `https://api.x.ai` | Root, `/v1`, or full‑endpoint URL. |
| `GROK_SEARCH_MODEL` | `grok-4-1-fast-reasoning` | Model name. |
| `GROK_SEARCH_WEB_SEARCH` | `true` | Offer `web_search` tool to Grok. |
| `GROK_SEARCH_X_SEARCH` | `false` | Offer `x_search` tool (X/Twitter) to Grok. |

Verified upstreams: **xAI** (`https://api.x.ai`, both tools), **Modelverse** (`https://api.modelverse.cn`, `x_search` depends on relay).

OAuth mode is a single-binary flow:

```bash
grok-search-rs login
grok-search-rs status
grok-search-rs logout
```

Then configure your MCP client with:

```toml
[mcp_servers.grok-search-rs]
command = "grok-search-rs"

[mcp_servers.grok-search-rs.env]
GROK_SEARCH_AUTH_MODE = "oauth"
GROK_SEARCH_MODEL = "grok-4.3"
GROK_SEARCH_WEB_SEARCH = "true"
```

OAuth mode reuses Hermes' xAI OAuth client id and stores `auth.json` locally. That may violate xAI terms or affect your account; do not share the token file. If xAI changes or blocks that OAuth flow, switch back to `api_key` mode.

### B. OpenAI‑compatible chat/completions

Activate by setting the URL **and** key while leaving `GROK_SEARCH_API_KEY` unset. Suitable for any OpenAI‑compatible relay (one‑api, vLLM, LiteLLM, marybrown, Perplexity‑style gateways, etc.).

| Variable | Default | Purpose |
|---|---|---|
| `OPENAI_COMPATIBLE_API_URL` | — | Root, `/v1`, or full‑endpoint URL. |
| `OPENAI_COMPATIBLE_API_KEY` | — | Bearer token for the relay. |
| `OPENAI_COMPATIBLE_MODEL` | falls back to `GROK_SEARCH_MODEL` | Model name to send. |

Notes:

- `GROK_SEARCH_WEB_SEARCH=true` (default) appends `tools:[{"type":"web_search"}]` to the payload. Relays that auto‑search server‑side simply ignore it.
- `GROK_SEARCH_X_SEARCH=true` is **silently ignored** on this transport (a one‑line stderr warning prints at startup). `x_search` only exists on the Responses API.
- Source extraction reads four parallel paths and de‑duplicates by URL: OpenAI `annotations[].url_citation`, Perplexity‑style `citations`, top‑level `search_sources[]`, and inline `[[n]](url)` markers.

### Tavily / Firecrawl (shared)

| Variable | Default | Purpose |
|---|---|---|
| `TAVILY_API_KEY` | — *(required for `web_fetch` / `web_map`)* | Tavily key. Comma‑separated list rotates round‑robin with failover on HTTP 401/403/429/432/433. |
| `TAVILY_API_URL` | `https://api.tavily.com` | Tavily base. |
| `GROK_SEARCH_EXTRA_SOURCES` | `3` | Extra Tavily sources after a Grok answer (`0` disables). |
| `GROK_SEARCH_FALLBACK_SOURCES` | `5` | Fallback source count when the AI step can't verify itself. |
| `FIRECRAWL_API_KEY` | unset | Enables Firecrawl as `web_fetch` / source fallback. Comma-separated list rotates round-robin with failover on HTTP 401/403/429/432/433. |
| `FIRECRAWL_API_URL` | `https://api.firecrawl.dev` | Firecrawl base. |
| `GROK_SEARCH_CACHE_SIZE` | `256` | Max cached `web_search` sessions. |
| `GROK_SEARCH_TIMEOUT_SECONDS` | `60` | HTTP timeout for all upstreams. |
| `GROK_SEARCH_PROXY` | `auto` | `auto` discovers env/system proxies and adopts one only after API reachability checks; `off` forces direct; a proxy URL forces that candidate. |
| `GROK_SEARCH_FETCH_MAX_CHARS` | unset | Default char cap on `web_fetch`. |
| `GROK_SEARCH_MAX_INLINE_SOURCES` | `5` | Max `web_search` sources carrying inline content; the rest are metadata‑only. |
| `GROK_SEARCH_RESPONSE_MAX_CHARS` | `60000` | Whole‑response char budget for `web_search`; over‑budget output is truncated tail‑first with `truncated: true`. |

### Source extraction (`web_fetch` specialists / `web_search` enrichment)

| Variable | Default | Purpose |
|---|---|---|
| `GITHUB_TOKEN` | unset | Authenticates GitHub issue/PR fetches (higher API rate limit; private repos). Specialist works unauthenticated but is rate‑limited. |
| `GROK_SEARCH_SOURCE_MAX_ANSWERS` | `5` | StackExchange answers rendered before folding. |
| `GROK_SEARCH_SOURCE_MAX_COMMENTS` | `30` | GitHub / StackExchange comments rendered before folding. |
| `GROK_SEARCH_ENRICH_CONCURRENCY` | `3` | Parallel source enrichments for `web_search` `include_content` (clamped 1..5). |
| `GROK_SEARCH_ENRICH_MAX_CHARS` | `15000` | Char cap per enriched source body. |

These specialists need **no Tavily/Firecrawl key** — they hit the public GitHub,
StackExchange, arXiv, and Wikipedia APIs directly. Tavily/Firecrawl are only used
for the generic fallback path.

### Selection rules at startup

1. If `GROK_SEARCH_AUTH_MODE=oauth` → **Responses** transport with the local OAuth token.
2. Else if `GROK_SEARCH_API_KEY` is set → **Responses** transport with a static Bearer key.
3. Else if both `OPENAI_COMPATIBLE_API_URL` and `OPENAI_COMPATIBLE_API_KEY` are set → **ChatCompletions** transport.
4. Else → server fails with a clear `MissingConfig` error.

### Global config file

Tired of duplicating `env` blocks across clients? Run `grok-search-rs --init` once to scaffold `<home>/.config/grok-search-rs/config.toml`, fill in your keys, and every client can shrink to `{"command": "grok-search-rs"}`.

| Path order | Location |
|---|---|
| 1 | `$GROK_SEARCH_CONFIG` (explicit override, any platform) |
| 2 | `$HOME/.config/grok-search-rs/config.toml` (Unix / macOS / Git Bash) |
| 3 | `%USERPROFILE%\.config\grok-search-rs\config.toml` (native Windows) |

**Precedence**: per‑client `env` **>** config file **>** built‑in defaults. File keys are lowercase `snake_case` (env `GROK_SEARCH_MODEL` → file `grok_model`). Unknown keys are rejected. Full reference: [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

---

## MCP Tools

| Tool | When to call it |
|---|---|
| `web_search` | Sourced summary for a topic. Sources cached for follow‑up. `response_format: "concise"` returns answer + metadata only; `"detailed"` inlines source text within the response budget. |
| `get_sources` | Re‑fetch sources of a previous `web_search` by `session_id`. Supports `offset` / `limit` pagination for large source sets. |
| `web_fetch` | Page content as clean Markdown. Specialist extractors for GitHub / StackExchange / arXiv / Wikipedia; generic Tavily → Firecrawl fallback otherwise. Returns `source_type` + `fallback_reason`. |
| `web_map` | Discover URLs on a domain via Tavily Map. |
| `doctor` | Live connectivity probe + redacted config. Run first when something looks off. |

---

## Build from source

```bash
git clone https://github.com/MosRat/GrokSearch-rs.git
cd GrokSearch-rs
cargo build --release
```

The binary lands at `target/release/grok-search-rs`. Point your MCP client's `command` at the absolute path.

---

## Development

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

More docs:

- [Configuration](docs/CONFIGURATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Testing](docs/TESTING.md)

---

## ⭐ Star History

<a href="https://www.star-history.com/?repos=MosRat%2FGrokSearch-rs&type=Date">
  <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=MosRat/GrokSearch-rs&type=Date" />
</a>

---

## Acknowledgements

- Inspired by [GuDaStudio/GrokSearch](https://github.com/GuDaStudio/GrokSearch) — the original Python implementation that pioneered the Grok + Tavily + Firecrawl combo this project rewrites in Rust.
- Thanks to the [LinuxDo](https://linux.do) community for the discussions, feedback, and the prior art that inspired this rewrite.

## License

MIT — see [LICENSE](LICENSE).
