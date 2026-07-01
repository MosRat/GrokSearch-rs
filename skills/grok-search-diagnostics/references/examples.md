# Diagnostics Examples

## Quick Probe

```json
{}
```

Use this to confirm the MCP handler can run and report backend status. It is a
configuration/provider probe, not a replacement for checking whether a
background HTTP process is currently alive.

## Verbose Probe

```json
{"verbose":true}
```

Use verbose mode when investigating:

- missing API keys
- provider wiring
- timeout or proxy behavior
- academic PDF cache behavior
- LLM progressive reading configuration
- response and fetch limits
- debug logging status
- URL policy failures
- HTTP MCP bind/auth/CORS configuration

## Reading Results

Secrets should appear as `set` or `unset`, never raw values. URLs and proxy settings should be redacted when credentials or query tokens are present.

If a provider is unreachable, check the corresponding config key, endpoint override, proxy, timeout, and network access before changing tool parameters.

If a tool reports missing capability, confirm whether the provider is intentionally optional or disabled.

If a tool call fails but `doctor` looks healthy, inspect the tool's own output
diagnostics next: PDF tools expose `pdf_cache`; LLM structure/artifact tools
expose processing/vision reports; HTTP MCP service health is checked through
`/healthz` and `mcp-service status`.

## HTTP MCP Checks

For local Streamable HTTP MCP, start the server explicitly:

```bash
grok-search-rs mcp-http --bind 127.0.0.1:8787 --path /mcp
```

For a persistent current-user background endpoint, install and manage the
current binary as a service:

```bash
grok-search-rs mcp-service install --bind 127.0.0.1:8787 --path /mcp
grok-search-rs mcp-service status
grok-search-rs mcp-service stop
grok-search-rs mcp-service uninstall
```

Use `mcp-http` for foreground logs and `mcp-service` when the endpoint should
survive terminal exit. `install` starts by default and reports `/healthz`;
`--no-start` only registers the service. The installer copies the current
binary to a managed user bin directory and updates that copy when the current
version is newer. On Linux it attempts `loginctl enable-linger <user>` so the
service can continue after logout; if that warning appears, run the printed
command manually.

`GET /healthz` is intentionally small and does not reveal secrets. MCP requests
go to the configured path, usually `/mcp`.

`mcp-service status` reports the user-service controller state. It does not
prove every provider credential is valid; use `doctor` for that. Conversely,
`doctor` can succeed while a background service is not installed or not
running.

If HTTP MCP returns 401, set `Authorization: Bearer <mcp_http_auth_token>` or
unset the token for loopback-only local use. If startup fails for `0.0.0.0` or
another non-loopback bind, configure `mcp_http_auth_token` first. If browser
clients fail before the MCP request reaches the server, check
`mcp_http_allow_origin`; CORS is disabled unless one explicit origin is set.

## Academic PDF And LLM Checks

For slow or flaky PDF reads, inspect the tool output first. `academic_pdf_*`
responses may include `pdf_cache` with `hit`, `stored`, `attempts`,
`backoff_ms`, `download_elapsed_ms`, and `warnings`. Warnings can include the
adaptive `download_plan`, the final `download_strategy`, and per-attempt timing.

Use `doctor` verbose output to confirm:

- `academic_pdf_cache_enabled`, path, TTL, entry limit, and byte limit.
- `progressive_cache_enabled`, path, TTL, and entry limit.
- `llm_provider`, `llm_base_url`, `llm_model`, and whether the LLM key is set.

Use `cache_policy:"refresh"` to force a new PDF download and cache overwrite.
Use `cache_policy:"bypass"` to ignore the PDF bytes cache while diagnosing
cache corruption or cold network performance. Keep `cache_policy:"auto"` for
normal usage.

`academic_pdf_artifacts` uses `vision_profile:"auto"` by default. If an LLM key
is set, it may run the multimodal artifact pass and write `vision` diagnostics;
if no key is set, it stays off without failing. Use `vision_profile:"off"` to
isolate deterministic pdf_oxide image/table extraction, or
`vision_cache_policy:"refresh"` to force a fresh LLM artifact pass.
