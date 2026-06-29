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
- response and fetch limits
- debug logging status
- URL policy failures

## Reading Results

Secrets should appear as `set` or `unset`, never raw values. URLs and proxy settings should be redacted when credentials or query tokens are present.

If a provider is unreachable, check the corresponding config key, endpoint override, proxy, timeout, and network access before changing tool parameters.

If a tool reports missing capability, confirm whether the provider is intentionally optional or disabled.
