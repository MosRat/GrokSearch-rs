#!/usr/bin/env bash
# Full local verification for the multi-crate workspace.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

if command -v cargo >/dev/null 2>&1; then
  CARGO=(cargo)
elif command -v cargo.exe >/dev/null 2>&1; then
  CARGO=(cargo.exe)
elif command -v rtk >/dev/null 2>&1; then
  CARGO=(rtk cargo)
else
  echo "cargo not found on PATH" >&2
  exit 127
fi

echo "==> cargo metadata"
"${CARGO[@]}" metadata --no-deps --format-version 1 >/dev/null

echo "==> cargo fmt"
"${CARGO[@]}" fmt --check

echo "==> cargo clippy"
"${CARGO[@]}" clippy --workspace --all-targets -- -D warnings

echo "==> cargo test"
"${CARGO[@]}" test --workspace

echo "==> cargo build --release"
"${CARGO[@]}" build --release

echo "workspace check passed"
