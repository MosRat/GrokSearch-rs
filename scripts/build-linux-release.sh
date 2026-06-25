#!/usr/bin/env bash
# Build Linux release binaries as static musl targets via Zig.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v zig >/dev/null 2>&1; then
  echo "zig not found on PATH; install Zig before building Linux release binaries" >&2
  exit 127
fi

if ! cargo zigbuild --version >/dev/null 2>&1; then
  echo "cargo-zigbuild not found; install with: cargo install cargo-zigbuild" >&2
  exit 127
fi

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
  targets=(
    x86_64-unknown-linux-musl
    aarch64-unknown-linux-musl
  )
fi

for target in "${targets[@]}"; do
  echo "==> cargo zigbuild --release --target ${target}"
  cargo zigbuild --release --target "$target" --locked
done
