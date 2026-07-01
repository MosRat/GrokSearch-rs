# Diagnostics Examples

## Quick Probe

```json
{}
```

Use this to confirm the server can run and report backend status.

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

## Reading Results

Secrets should appear as `set` or `unset`, never raw values. URLs and proxy settings should be redacted when credentials or query tokens are present.

If a provider is unreachable, check the corresponding config key, endpoint override, proxy, timeout, and network access before changing tool parameters.

If a tool reports missing capability, confirm whether the provider is intentionally optional or disabled.

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
