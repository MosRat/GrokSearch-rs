#!/usr/bin/env bash
# Package repo-local GrokSearch-rs skills as a release asset.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$REPO_ROOT/dist/grok-search-rs-skills.tar.gz}"

cd "$REPO_ROOT"

[ -d skills ] || {
  echo "skills directory not found" >&2
  exit 1
}
[ -f scripts/install-skills.sh ] || {
  echo "scripts/install-skills.sh not found" >&2
  exit 1
}

found=0
for skill in skills/*; do
  if [ -d "$skill" ] && [ -f "$skill/SKILL.md" ]; then
    found=$((found + 1))
  fi
done
[ "$found" -gt 0 ] || {
  echo "no skills with SKILL.md found" >&2
  exit 1
}

mkdir -p "$(dirname "$OUT")"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

mkdir -p "$tmp/scripts"
cp -R skills "$tmp/skills"
cp scripts/install-skills.sh "$tmp/scripts/install-skills.sh"
chmod +x "$tmp/scripts/install-skills.sh"

tar -czf "$OUT" -C "$tmp" skills scripts/install-skills.sh
echo "Wrote $OUT"
