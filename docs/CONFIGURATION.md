# Configuration

GrokSearch-rs reads configuration from two sources, merged with the following precedence:

1. **Process environment variables** (highest — what your MCP client passes in `env`).
2. **Global TOML config file** — `$GROK_SEARCH_CONFIG` if set, otherwise `<home>/.config/grok-search-rs/config.toml` on every platform. `<home>` is `$HOME` on Unix / Git Bash, `%USERPROFILE%` on native Windows shells (PowerShell, cmd).
3. **Built-in defaults** (lowest).

The config file is optional; missing files are skipped silently. See the [Config file](#config-file) section below for the TOML schema. The AI provider contract is intentionally narrow: configure a Grok/OpenAI-compatible root URL and the server calls `/v1/responses`.

Run `grok-search-rs init` to create the shared config if it is missing and keep agent-specific MCP entries thin. The recommended split is:

- Global config: provider URLs, models, source limits, proxy settings, and API keys.
- Agent config: only `type = "stdio"` and `command = "grok-search-rs"`; add `GROK_SEARCH_CONFIG` only when you intentionally use a non-default config path.

The same global config is used by both MCP mode and direct CLI tool calls such
as `grok-search-rs doctor`, `grok-search-rs web-search "query"`,
`grok-search-rs repo-metadata --url "https://github.com/owner/repo"`, and
`grok-search-rs academic search "query"`. Direct CLI tool calls print pretty
JSON by default; pass `--compact` for single-line JSON. Use
`grok-search-rs doctor --verbose` for detailed limits, logging status,
provider wiring, and URL policy diagnostics.

## Grok Responses

| Variable | Default | Description |
|---|---|---|
| `GROK_SEARCH_AUTH_MODE` | `api_key` | `api_key` uses `GROK_SEARCH_API_KEY`; `oauth` uses the local token file created by `grok-search-rs login`. |
| `GROK_SEARCH_API_KEY` | required in `api_key` mode | Bearer token for the configured Grok-compatible gateway. |
| `GROK_SEARCH_AUTH_FILE` | `<home>/.config/grok-search-rs/auth.json` | Optional OAuth token file override. |
| `GROK_SEARCH_URL` | `https://api.x.ai` | Root URL, `/v1` base URL, or endpoint-like URL. The service normalizes it to a `/v1` base. |
| `GROK_SEARCH_MODEL` | `grok-4-1-fast-reasoning` | Model sent in the Responses payload. |
| `GROK_SEARCH_WEB_SEARCH` | `true` | Sends Responses `{"type":"web_search"}`. |
| `GROK_SEARCH_X_SEARCH` | `false` | Sends Responses `{"type":"x_search"}` only when enabled. |

Boolean values accept `1`, `true`, or `yes` as enabled. Any other value is treated as disabled.

Example:

```bash
GROK_SEARCH_API_KEY=...
GROK_SEARCH_URL=https://api.modelverse.cn
GROK_SEARCH_MODEL=grok-4-1-fast-reasoning
GROK_SEARCH_X_SEARCH=false
```

The example above calls `https://api.modelverse.cn/v1/responses`.

### OAuth mode

OAuth mode keeps the normal Responses payload and only changes where the Bearer token comes from. The binary handles login and MCP stdio; it does not start a background HTTP proxy.

```bash
grok-search-rs login
grok-search-rs status
grok-search-rs logout
```

`login` opens xAI OAuth in a browser, listens once on `http://127.0.0.1:56121/callback`, and writes `access_token`, `refresh_token`, `id_token`, `token_endpoint`, `base_url`, and `last_refresh` to `auth.json`. `status` prints token presence, expiry, and the auth file path without printing the token. `logout` removes the local auth file.

OAuth mode reuses Hermes' xAI OAuth client id. This may violate xAI terms or create account risk, and Windows stores the token as a normal local file. Do not share the token file.

Minimal Codex config. Put OAuth mode and model selection in the global config:

```toml
[mcp_servers.grok-search-rs]
type = "stdio"
command = "grok-search-rs"
```

## Tavily

| Variable | Default | Description |
|---|---|---|
| `TAVILY_API_KEY` | unset | Enables Tavily-backed source enrichment, fallback, fetch, and map. Accepts a single key or a comma-separated list (`tvly-a,tvly-b`); multiple keys rotate round-robin per request, with automatic failover to the next key on key-scoped errors (HTTP 401/403/429/432/433). |
| `TAVILY_API_URL` | `https://api.tavily.com` | Tavily API base URL. |
| `TAVILY_ENABLED` | `true` | Optional override. Set to `false` only when you want to disable Tavily even if `TAVILY_API_KEY` is configured. |
| `GROK_SEARCH_EXTRA_SOURCES` | `3` | Adds Tavily enrichment sources after a verifiable Grok result; Firecrawl can fallback if Tavily returns none. Set `0` to disable enrichment. |
| `GROK_SEARCH_FALLBACK_SOURCES` | `5` | Number of fallback sources to cache when Grok is unverifiable. |

## Firecrawl

| Variable | Default | Description |
|---|---|---|
| `FIRECRAWL_API_KEY` | unset | Enables Firecrawl fallback for `web_fetch` and supplemental fallback sources. Accepts a single key or a comma-separated list (`fc-a,fc-b`); multiple keys rotate round-robin per request, with automatic failover to the next key on key-scoped errors (HTTP 401/403/429/432/433). |
| `FIRECRAWL_API_URL` | `https://api.firecrawl.dev` | Firecrawl API base URL, normalized to `/v1`. |
| `FIRECRAWL_ENABLED` | `true` | Optional override. Set to `false` to disable Firecrawl even if a key is configured. |

## Cache

| Variable | Default | Description |
|---|---|---|
| `GROK_SEARCH_CACHE_SIZE` | `256` | Maximum cached search sessions for `get_sources`. |
| `GROK_SEARCH_TIMEOUT_SECONDS` | `60` | HTTP timeout for Grok, Tavily, and Firecrawl requests. |
| `GROK_SEARCH_PROXY` | `auto` | Proxy mode. `auto` discovers environment/system proxies and uses one only after API reachability checks; `off` forces direct; a URL such as `http://127.0.0.1:7890` or `socks5://127.0.0.1:1080` forces that candidate. |
| `GROK_SEARCH_FETCH_MAX_CHARS` | unset | Default character cap on `web_fetch` content. Overridden per call by `max_chars`. Unset means no truncation. |
| `GROK_SEARCH_MAX_RESPONSE_BYTES` | `10485760` | Global upstream HTTP response body byte cap before JSON/PDF/text parsing. |
| `GROK_SEARCH_DEBUG_LOG_PATH` | unset | Optional JSONL debug log path. Disabled when unset; payloads are redacted before writing. |

## Proxy

Proxy discovery is cross-platform and conservative: environment variables win first (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`, plus lowercase forms), then OS settings are considered. Windows reads the current user's Internet Settings, macOS parses `scutil --proxy`, and Linux checks GNOME `gsettings` before KDE's `kioslaverc`. `NO_PROXY` / `no_proxy` bypasses environment-discovered proxies for matching API hosts.

`auto` never adopts a discovered proxy just because it exists. At startup, `grok-search-rs` probes the configured Grok/OpenAI-compatible, Tavily, and Firecrawl API bases with lightweight `HEAD` requests (falling back to `GET`); any HTTP response counts as network reachability. If the proxy path cannot reach all probeable APIs, the server falls back to direct connections and reports the decision in `doctor.proxy`. PAC/WPAD scripts are detected where possible but not executed.

## Source extraction

Specialist `web_fetch` extractors (GitHub, StackExchange, arXiv, Wikipedia),
`repo_metadata`, and `web_search` inline enrichment. The specialists call public APIs directly — no
Tavily/Firecrawl key required.

| Variable | Default | Description |
|---|---|---|
| `GITHUB_TOKEN` | unset | GitHub token for issue/PR/repo metadata fetches. If unset, GrokSearch-rs tries `gh auth token`; otherwise anonymous works but is capped at ~60 req/hr. |
| `HF_TOKEN` / `HUGGINGFACE_TOKEN` | unset | Optional Hugging Face token for `repo_metadata` model/dataset metadata and card fetches. These are read directly from process env and are not global config fields or doctor probes. |
| `GROK_SEARCH_SOURCE_MAX_ANSWERS` | `5` | StackExchange answers rendered before the "more answers" fold. |
| `GROK_SEARCH_SOURCE_MAX_COMMENTS` | `30` | GitHub / StackExchange comments rendered before folding. |
| `GROK_SEARCH_ENRICH_CONCURRENCY` | `3` | Parallel source enrichments when `web_search` is called with `include_content: true`. Clamped to `1..=5`. |
| `GROK_SEARCH_ENRICH_MAX_CHARS` | `15000` | Character cap per enriched source body. |

`repo_metadata` supports GitHub repositories and Hugging Face models/datasets.
It returns metadata only by default. GitHub README text is fetched only with
`include_readme`; Hugging Face model/dataset card text is fetched only with
`include_card`. Hugging Face Spaces are intentionally rejected in this version.

## Response budget

Caps the size of a single `web_search` response so large source sets cannot
blow past MCP client context limits. The session cache always keeps full
content; truncated sources carry a note pointing at `web_fetch(url)` /
`get_sources(session_id)` for recovery.

| Variable | Default | Description |
|---|---|---|
| `GROK_SEARCH_MAX_INLINE_SOURCES` | `5` | Maximum sources that carry inline `content` per `web_search` response, clamped to `0..20`; `0` returns metadata only. |
| `GROK_SEARCH_RESPONSE_MAX_CHARS` | `60000` | Whole-response character budget (answer + per-source metadata and inline content). Over-budget responses truncate inline content tail-first, then drop trailing sources (always keeping at least one) and set `truncated: true`. |

## Academic search

The `academic_*` MCP tools are independent of `web_*` and focus on computer-science literature. They reuse the same HTTP client, timeout, proxy, redaction, and doctor diagnostics pipeline as the rest of the server.

| Variable | Default | Description |
|---|---|---|
| `GROK_SEARCH_ACADEMIC_ENABLED` | `true` | Enables `academic_search`, `academic_get`, `academic_citations`, and the academic PDF tools. |
| `GROK_SEARCH_ACADEMIC_EMAIL` | unset | Contact email for Unpaywall and polite OpenAlex/Crossref usage. Unpaywall is skipped when absent. Legacy `UNPAYWALL_EMAIL` is also accepted. |
| `SEMANTIC_SCHOLAR_API_KEY` | unset | Optional Semantic Scholar key. When unset, anonymous Graph API requests are used. |
| `OPENALEX_API_KEY` / `GROK_SEARCH_OPENALEX_API_KEY` | unset | Optional OpenAlex key. Recommended for more reliable OpenAlex search, citation, full-text metadata, and enrichment calls. |
| `GROK_SEARCH_ACADEMIC_SCIHUB_ENABLED` | `false` | Explicit opt-in for Sci-Hub fallback in academic PDF resolution. It is never used by default. |
| `GROK_SEARCH_ACADEMIC_SCIHUB_BASE_URL` | unset | Sci-Hub base URL, only used when Sci-Hub fallback is enabled. User/password components are redacted in Debug and doctor output. |
| `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ENABLED` | `true` | Enables IEEE/ACM institutional PDF fallback for academic PDF tools; automatically disables itself when no usable route is found. |
| `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS` | `false` | Allows invalid TLS certificates only for private/local IEEE/ACM institutional fallback routes. Public routes require HTTPS validation. |
| `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_PROBE` | `true` | Probes direct and discovered proxy routes for IEEE/ACM access before using the fallback. |
| `GROK_SEARCH_ACADEMIC_MAX_PDF_BYTES` | `52428800` | Maximum PDF bytes downloaded for academic PDF tools. |
| `GROK_SEARCH_ACADEMIC_PDF_MAX_CHARS` | unset | Character cap for parsed PDF output. Falls back to `GROK_SEARCH_FETCH_MAX_CHARS`, then `200000`. |

Academic PDF tools are independent public entry points over shared internals:
`academic_pdf_read` returns processed text, `academic_pdf_structure` returns an
LLM-assisted progressive reading structure, `academic_pdf_artifacts` writes
image/table artifacts and manifests, and `academic_pdf_download` saves the raw
PDF. Each accepts exactly one locator: `identifier`, `url`, or `pdf_url`.
Legacy `academic_read`, `academic_parse_pdf`, and `academic_download_pdf`
remain compatible aliases for older clients.

PDF text flows through a local pipeline: raw `pdf_oxide` extraction, text
signal analysis, and cleanup. The default `text_mode` is `clean`; use
`text_mode="none"` for raw extraction, or `include_raw_content=true` when
comparing raw and processed text through `academic_pdf_read`. Artifact
extraction is partial: `academic_pdf_artifacts` can export filtered bitmap
XObjects as PNG files with `images.json`, and detected tables as Markdown
snippets with `tables.json`; it does not reconstruct semantic vector figures
or OCR scanned pages. Material link extraction is local URL/text
classification only and does not fetch GitHub, Hugging Face, dataset, model,
demo, or project URLs.

PDF downloads use `cache_policy` per call. `auto` reads and writes the PDF
bytes cache, `refresh` redownloads and overwrites it, and `bypass` skips it.
Cold downloads use retry backoff and an adaptive full/range strategy; repeated
calls for the same resolved PDF normally avoid the network entirely.

`academic_pdf_structure` uses the LLM config below only when a progressive
structure is requested. The public tool exposes `profile` (`fast`, `balanced`,
or `strict`) instead of chunk sizes, prompt profiles, or concurrency. The
progressive cache stores the typed JSON structure keyed by PDF/input/strategy
hashes; it does not store API keys, raw prompts, raw model responses, or raw
PDF bytes.

Academic provider calls use conservative built-in stability guards. arXiv API requests are globally spaced by 3 seconds and retry `429` responses. OpenAlex requests retry transient `502`/`503`/`504` gateway failures, and broad `sort_by=date` searches without an explicit year filter avoid OpenAlex's `publication_date:desc` server-side sort to reduce 504 slow-query failures.

## Config file

Drop a TOML file at `<home>/.config/grok-search-rs/config.toml` (or any path pointed to by `GROK_SEARCH_CONFIG`) to set defaults once and skip the per-client `env` block. Process env still wins, so individual clients can override any field at runtime.

Resolved per platform:

- **macOS / Linux**: `$HOME/.config/grok-search-rs/config.toml` — e.g. `/Users/alice/.config/grok-search-rs/config.toml`.
- **Windows (PowerShell / cmd)**: `%USERPROFILE%\.config\grok-search-rs\config.toml` — e.g. `C:\Users\chen\.config\grok-search-rs\config.toml`.
- **Windows (Git Bash / MSYS)**: same as Unix — `$HOME/.config/grok-search-rs/config.toml`.

`grok-search-rs init` picks the right path automatically; no platform-specific shell setup required.

### Scaffolding config and agent entries - `init`

```bash
grok-search-rs init
```

This writes an annotated template at the resolved config path with **every key commented out** when the file does not exist. The scaffold is identical in behavior to "no config file" until you uncomment lines, so it never silently changes runtime behavior.

The same command also keeps thin MCP entries/snippets in sync:

- Codex: updates `$CODEX_HOME/config.toml` or `<home>/.codex/config.toml`.
- Claude Code: runs `claude mcp add-json grok-search-rs --scope user ...` when the `claude` CLI is available, otherwise prints the command to run.
- Other MCP clients: writes reusable snippets in `<home>/.config/grok-search-rs/agent-snippets/` (`mcp.json`, `cursor.json`, `vscode.json`, `windsurf.json`, `codex.toml`, `claude-code.json`).

The generated agent entries intentionally do not include API keys or model/provider settings. Re-running `init` is idempotent: it refreshes the managed thin entries and leaves an existing global config untouched.

Use `--target codex`, `--target claude-code`, or `--target snippets` to refresh
only one integration. Use `--dry-run` to preview paths and commands without
writing files or invoking the Claude Code CLI.

Minimal generated Codex entry:

```toml
[mcp_servers.grok-search-rs]
type = "stdio"
command = "grok-search-rs"
```

If you launch `init` with `GROK_SEARCH_CONFIG` pointing at a custom path, the generated agent entries include only that forwarding env var:

```toml
[mcp_servers.grok-search-rs.env]
GROK_SEARCH_CONFIG = "/absolute/path/to/config.toml"
```

### Why two casings?

Env vars use `UPPER_CASE` because that is the Unix shell tradition (`PATH`, `HOME`, `LANG`, `AWS_REGION` …). TOML files use lowercase `snake_case` because that is the Rust ecosystem convention (`Cargo.toml`, `pyproject.toml`, Codex `~/.codex/config.toml`). `grok-search-rs` follows each convention in its native context. Mapping rule for the table below: drop the `GROK_SEARCH_` prefix where present, then lowercase the rest.

Unknown keys are rejected by the loader — typos surface as parse errors instead of silently dropping.

<!-- config-schema:start -->
### Generated config reference

This section is generated from the Rust config schema in `grok-search-config`. Update the schema first, then refresh this block.

#### Grok Responses

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `grok_api_url` | `GROK_SEARCH_URL` | https://api.x.ai | Root URL, /v1 base URL, or full endpoint. Normalized to /v1 for Responses calls. |
| `grok_api_key` | `GROK_SEARCH_API_KEY` | unset | Bearer token for the configured Grok/xAI-compatible Responses gateway. |
| `grok_auth_mode` | `GROK_SEARCH_AUTH_MODE` | api_key | Authentication mode. Use api_key for static keys or oauth for the local OAuth token file. |
| `grok_auth_file` | `GROK_SEARCH_AUTH_FILE` | default auth.json next to config.toml | Optional OAuth token file override. |
| `grok_model` | `GROK_SEARCH_MODEL` | grok-4-1-fast-reasoning | Model sent in the Responses payload. |
| `web_search_enabled` | `GROK_SEARCH_WEB_SEARCH` | true | Enables the upstream Responses web_search tool. |
| `x_search_enabled` | `GROK_SEARCH_X_SEARCH` | false | Enables the upstream Responses x_search tool when the gateway supports it. |
#### Source providers

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `tavily_api_url` | `TAVILY_API_URL` | https://api.tavily.com | Tavily API base URL. |
| `tavily_api_key` | `TAVILY_API_KEY` | unset | Tavily key for enrichment, fallback, fetch, and map. |
| `tavily_enabled` | `TAVILY_ENABLED` | true | Optional Tavily enable override. |
| `firecrawl_api_url` | `FIRECRAWL_API_URL` | https://api.firecrawl.dev | Firecrawl API base URL. Normalized to /v1. |
| `firecrawl_api_key` | `FIRECRAWL_API_KEY` | unset | Firecrawl key for fetch fallback and supplemental fallback sources. |
| `firecrawl_enabled` | `FIRECRAWL_ENABLED` | true | Optional Firecrawl enable override. |
| `default_extra_sources` | `GROK_SEARCH_EXTRA_SOURCES` | 3 | Adds Tavily enrichment sources after a verifiable Grok result. |
| `fallback_sources` | `GROK_SEARCH_FALLBACK_SOURCES` | 5 | Number of fallback sources to cache when Grok is unverifiable. |
#### Runtime limits

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `fetch_max_chars` | `GROK_SEARCH_FETCH_MAX_CHARS` | unset | Default character cap on web_fetch content. Unset means no truncation. |
| `cache_size` | `GROK_SEARCH_CACHE_SIZE` | 256 | Maximum cached web_search sessions for get_sources. |
| `timeout_seconds` | `GROK_SEARCH_TIMEOUT_SECONDS` | 60 | HTTP timeout in seconds for upstream requests. |
| `proxy` | `GROK_SEARCH_PROXY` | auto | Proxy mode: auto, off, or an explicit proxy URL. |
| `max_response_bytes` | `GROK_SEARCH_MAX_RESPONSE_BYTES` | 10485760 | Global upstream response body byte cap before parsing or trimming. |
| `debug_log_path` | `GROK_SEARCH_DEBUG_LOG_PATH` | unset | Optional JSONL debug log path. |
#### OpenAI-compatible transport

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `openai_compatible_api_url` | `OPENAI_COMPATIBLE_API_URL` | unset | OpenAI-compatible chat-completions gateway base URL. |
| `openai_compatible_api_key` | `OPENAI_COMPATIBLE_API_KEY` | unset | OpenAI-compatible gateway bearer token. |
| `openai_compatible_model` | `OPENAI_COMPATIBLE_MODEL` | falls back to grok_model | OpenAI-compatible model name. |
#### LLM providers

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `llm_provider` | `GROK_SEARCH_LLM_PROVIDER` | minimax | Default LLM provider used by experimental PDF progressive reading. |
| `llm_api_key` | `GROK_SEARCH_LLM_API_KEY`<br>`ANTHROPIC_API_KEY`<br>`MINIMAX_API_KEY` | unset | LLM API key for experimental PDF progressive reading. |
| `llm_base_url` | `GROK_SEARCH_LLM_BASE_URL`<br>`ANTHROPIC_BASE_URL` | https://api.minimaxi.com/anthropic | Anthropic-compatible base URL for experimental PDF progressive reading. |
| `llm_model` | `GROK_SEARCH_LLM_MODEL`<br>`ANTHROPIC_MODEL` | MiniMax-M3 | Default LLM model for experimental PDF progressive reading. |
| `llm_auth_scheme` | `GROK_SEARCH_LLM_AUTH_SCHEME` | bearer | Authentication scheme for Anthropic-compatible LLM calls. |
#### Source extraction

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `github_token` | `GITHUB_TOKEN` | unset | GitHub token for issue, PR, and repo metadata fetches. |
| `source_max_answers` | `GROK_SEARCH_SOURCE_MAX_ANSWERS` | 5 | StackExchange answers rendered before folding. |
| `source_max_comments` | `GROK_SEARCH_SOURCE_MAX_COMMENTS` | 30 | GitHub and StackExchange comments rendered before folding. |
| `enrich_concurrency` | `GROK_SEARCH_ENRICH_CONCURRENCY` | 3 | Parallel source enrichments when web_search includes content. |
| `enrich_max_chars` | `GROK_SEARCH_ENRICH_MAX_CHARS` | 15000 | Character cap per enriched source body. |
| `max_inline_sources` | `GROK_SEARCH_MAX_INLINE_SOURCES` | 5 | Maximum sources carrying inline content per web_search response. |
| `response_max_chars` | `GROK_SEARCH_RESPONSE_MAX_CHARS` | 60000 | Whole-response character budget for web_search. |
#### Academic and social search

| TOML key | Env aliases | Default | Description |
|---|---|---|---|
| `academic_enabled` | `GROK_SEARCH_ACADEMIC_ENABLED` | true | Enables the academic_* MCP tools. |
| `academic_email` | `GROK_SEARCH_ACADEMIC_EMAIL`<br>`UNPAYWALL_EMAIL` | unset | Contact email for Unpaywall and polite academic API usage. |
| `semantic_scholar_api_key` | `SEMANTIC_SCHOLAR_API_KEY` | unset | Optional Semantic Scholar API key. |
| `openalex_api_key` | `OPENALEX_API_KEY`<br>`GROK_SEARCH_OPENALEX_API_KEY` | unset | Optional OpenAlex key. Comma-separated lists rotate keys round-robin. |
| `zhihu_api_key` | `ZHIHU_ACCESS_SECRET`<br>`ZHIHU_API_KEY` | unset | Zhihu OpenAPI Access Secret for zhihu_search. |
| `zhihu_openapi_base_url` | `ZHIHU_OPENAPI_BASE_URL` | https://developer.zhihu.com | Zhihu OpenAPI base URL. |
| `zhihu_search_url` | `ZHIHU_ZHIHU_SEARCH_URL` | unset | Full Zhihu search endpoint override. |
| `academic_scihub_enabled` | `GROK_SEARCH_ACADEMIC_SCIHUB_ENABLED` | false | Explicit opt-in for Sci-Hub fallback. Legal risk varies by jurisdiction. |
| `academic_scihub_base_url` | `GROK_SEARCH_ACADEMIC_SCIHUB_BASE_URL` | unset | Sci-Hub base URL, used only when academic_scihub_enabled is true. |
| `academic_institutional_enabled` | `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ENABLED` | true | Enables IEEE/ACM institutional PDF fallback. |
| `academic_institutional_accept_invalid_certs` | `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_ACCEPT_INVALID_CERTS` | false | Allows invalid TLS certificates only for private institutional fallback routes. |
| `academic_institutional_probe` | `GROK_SEARCH_ACADEMIC_INSTITUTIONAL_PROBE` | true | Probes direct and discovered proxy routes for IEEE/ACM access. |
| `academic_max_pdf_bytes` | `GROK_SEARCH_ACADEMIC_MAX_PDF_BYTES` | 52428800 | Maximum PDF bytes downloaded for academic PDF read, parse, and download flows. |
| `academic_pdf_max_chars` | `GROK_SEARCH_ACADEMIC_PDF_MAX_CHARS` | unset | Character cap for parsed PDF output. |
| `academic_pdf_cache_enabled` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_ENABLED` | true | Enables persistent PDF bytes cache for academic PDF tools. |
| `academic_pdf_cache_path` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH` | default academic-pdf-cache.redb next to config.toml | Persistent PDF bytes cache path for academic PDF tools. |
| `academic_pdf_cache_ttl_seconds` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_TTL_SECONDS` | 604800 | Seconds before cached PDF bytes expire. |
| `academic_pdf_cache_max_entries` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_MAX_ENTRIES` | 128 | Maximum cached PDF entries retained after writes. |
| `academic_pdf_cache_max_bytes` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_MAX_BYTES` | 2147483648 | Maximum total cached PDF bytes retained after writes. |
| `progressive_cache_enabled` | `GROK_SEARCH_PROGRESSIVE_CACHE_ENABLED` | true | Enables persistent KV cache for LLM progressive PDF reading structures. |
| `progressive_cache_path` | `GROK_SEARCH_PROGRESSIVE_CACHE_PATH` | default progressive-cache.redb next to config.toml | Persistent KV cache path for LLM progressive PDF reading structures. |
| `progressive_cache_ttl_seconds` | `GROK_SEARCH_PROGRESSIVE_CACHE_TTL_SECONDS` | 2592000 | Seconds before progressive reading cache entries expire. |
| `progressive_cache_max_entries` | `GROK_SEARCH_PROGRESSIVE_CACHE_MAX_ENTRIES` | 512 | Maximum progressive reading cache entries retained after writes. |
| `progressive_default_model` | `GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL` | MiniMax-M3 | Default model for LLM progressive PDF reading when the tool does not pass one. |
| `progressive_default_profile` | `GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE` | balanced | Default profile for LLM progressive PDF reading. |

<!-- config-schema:end -->

| TOML key | Env equivalent |
|---|---|
| `grok_api_url` | `GROK_SEARCH_URL` |
| `grok_api_key` | `GROK_SEARCH_API_KEY` |
| `grok_auth_mode` | `GROK_SEARCH_AUTH_MODE` |
| `grok_auth_file` | `GROK_SEARCH_AUTH_FILE` |
| `grok_model` | `GROK_SEARCH_MODEL` |
| `web_search_enabled` | `GROK_SEARCH_WEB_SEARCH` |
| `x_search_enabled` | `GROK_SEARCH_X_SEARCH` |
| `tavily_api_url` | `TAVILY_API_URL` |
| `tavily_api_key` | `TAVILY_API_KEY` |
| `tavily_enabled` | `TAVILY_ENABLED` |
| `firecrawl_api_url` | `FIRECRAWL_API_URL` |
| `firecrawl_api_key` | `FIRECRAWL_API_KEY` |
| `firecrawl_enabled` | `FIRECRAWL_ENABLED` |
| `default_extra_sources` | `GROK_SEARCH_EXTRA_SOURCES` |
| `fallback_sources` | `GROK_SEARCH_FALLBACK_SOURCES` |
| `fetch_max_chars` | `GROK_SEARCH_FETCH_MAX_CHARS` |
| `cache_size` | `GROK_SEARCH_CACHE_SIZE` |
| `timeout_seconds` | `GROK_SEARCH_TIMEOUT_SECONDS` |
| `proxy` | `GROK_SEARCH_PROXY` |
| `github_token` | `GITHUB_TOKEN` |
| `source_max_answers` | `GROK_SEARCH_SOURCE_MAX_ANSWERS` |
| `source_max_comments` | `GROK_SEARCH_SOURCE_MAX_COMMENTS` |
| `enrich_concurrency` | `GROK_SEARCH_ENRICH_CONCURRENCY` |
| `enrich_max_chars` | `GROK_SEARCH_ENRICH_MAX_CHARS` |
| `max_inline_sources` | `GROK_SEARCH_MAX_INLINE_SOURCES` |
| `response_max_chars` | `GROK_SEARCH_RESPONSE_MAX_CHARS` |
| `max_response_bytes` | `GROK_SEARCH_MAX_RESPONSE_BYTES` |
| `debug_log_path` | `GROK_SEARCH_DEBUG_LOG_PATH` |
| `academic_enabled` | `GROK_SEARCH_ACADEMIC_ENABLED` |
| `academic_email` | `GROK_SEARCH_ACADEMIC_EMAIL` |
| `semantic_scholar_api_key` | `SEMANTIC_SCHOLAR_API_KEY` |
| `openalex_api_key` | `OPENALEX_API_KEY` or `GROK_SEARCH_OPENALEX_API_KEY` |
| `zhihu_api_key` | `ZHIHU_ACCESS_SECRET` or `ZHIHU_API_KEY` |
| `zhihu_openapi_base_url` | `ZHIHU_OPENAPI_BASE_URL` |
| `zhihu_search_url` | `ZHIHU_ZHIHU_SEARCH_URL` |
| `academic_scihub_enabled` | `GROK_SEARCH_ACADEMIC_SCIHUB_ENABLED` |
| `academic_scihub_base_url` | `GROK_SEARCH_ACADEMIC_SCIHUB_BASE_URL` |
| `academic_max_pdf_bytes` | `GROK_SEARCH_ACADEMIC_MAX_PDF_BYTES` |
| `academic_pdf_max_chars` | `GROK_SEARCH_ACADEMIC_PDF_MAX_CHARS` |
| `academic_pdf_cache_enabled` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_ENABLED` |
| `academic_pdf_cache_path` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_PATH` |
| `academic_pdf_cache_ttl_seconds` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_TTL_SECONDS` |
| `academic_pdf_cache_max_entries` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_MAX_ENTRIES` |
| `academic_pdf_cache_max_bytes` | `GROK_SEARCH_ACADEMIC_PDF_CACHE_MAX_BYTES` |
| `progressive_cache_enabled` | `GROK_SEARCH_PROGRESSIVE_CACHE_ENABLED` |
| `progressive_cache_path` | `GROK_SEARCH_PROGRESSIVE_CACHE_PATH` |
| `progressive_cache_ttl_seconds` | `GROK_SEARCH_PROGRESSIVE_CACHE_TTL_SECONDS` |
| `progressive_cache_max_entries` | `GROK_SEARCH_PROGRESSIVE_CACHE_MAX_ENTRIES` |
| `progressive_default_model` | `GROK_SEARCH_PROGRESSIVE_DEFAULT_MODEL` |
| `progressive_default_profile` | `GROK_SEARCH_PROGRESSIVE_DEFAULT_PROFILE` |

`semantic_scholar_api_key` is sent to Semantic Scholar as the `x-api-key`
header. Semantic Scholar allows 1 request per second per key across all
endpoints; GrokSearch-rs serializes Semantic Scholar calls with a built-in
1.1 second minimum interval to stay below that threshold.

`zhihu_api_key` is sent to Zhihu OpenAPI as a Bearer token for the
`zhihu_search` tool. `zhihu_search_url` overrides the full endpoint when set.

`progressive_cache_*` controls the local embedded KV store used by the
experimental LLM progressive PDF reading pass. The cache stores structured
progressive reading JSON keyed by PDF/input/strategy hashes, not raw PDF bytes
or provider credentials.

Example — minimum useful file:

```toml
grok_api_key   = "xai-..."
tavily_api_key = "tvly-..."
grok_model     = "grok-4-1-fast-reasoning"
```

Example — OAuth mode:

```toml
grok_auth_mode = "oauth"
grok_model     = "grok-4.3"
```

Example — full reference:

```toml
grok_api_url          = "https://api.x.ai"
grok_api_key          = "xai-..."
grok_auth_mode        = "api_key"
# grok_auth_file      = "C:\\Users\\chen\\.config\\grok-search-rs\\auth.json"
grok_model            = "grok-4-1-fast-reasoning"
web_search_enabled    = true
x_search_enabled      = false
tavily_api_url        = "https://api.tavily.com"
tavily_api_key        = "tvly-..."
tavily_enabled        = true
firecrawl_api_url     = "https://api.firecrawl.dev"
firecrawl_api_key     = "fc-..."
firecrawl_enabled     = true
default_extra_sources = 3
fallback_sources      = 5
fetch_max_chars       = 200000
max_response_bytes    = 10485760
debug_log_path        = "logs/grok-search-rs-debug.jsonl"
cache_size            = 256
timeout_seconds       = 60
proxy                 = "auto"
```
