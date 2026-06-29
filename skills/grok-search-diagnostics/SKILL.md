---
name: grok-search-diagnostics
description: Use GrokSearch-rs diagnostics for MCP server health checks, masked configuration review, provider wiring, connectivity debugging, response limits, logging status, and setup troubleshooting with the doctor tool.
---

# GrokSearch Diagnostics

Use this skill when GrokSearch-rs setup, provider wiring, or MCP tool availability needs troubleshooting.

## Workflow

1. Call `doctor` with default parameters for a quick health probe.
2. Call `doctor` with `verbose: true` when debugging limits, provider wiring, URL policy, logging, or masked configuration.
3. Treat diagnostics as safe to share only after confirming secrets are redacted.
4. Use the reported provider status to decide whether the issue is configuration, network, credentials, or a disabled capability.

Read `references/examples.md` for probe examples and interpretation notes.
