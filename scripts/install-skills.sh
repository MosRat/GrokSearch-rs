#!/usr/bin/env bash
# Install GrokSearch-rs repo-local skills into a local directory or agent skill home.

set -euo pipefail

REPO="${GROK_SEARCH_RS_REPO:-MosRat/GrokSearch-rs}"
VERSION="${GROK_SEARCH_RS_VERSION:-latest}"
TARGET="${GROK_SEARCH_SKILLS_TARGET:-current}"
DEST=""
SOURCE=""
OVERWRITE=1
DRY_RUN=0

usage() {
  cat <<'EOF'
Install GrokSearch-rs skills.

Usage:
  curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash -s -- [options]

Options:
  --target <current|codex|cc|claude-code|dir>
      Install target. Default: current.
      current     -> ./skills
      codex       -> ${CODEX_HOME:-$HOME/.codex}/skills
      cc          -> ${CLAUDE_HOME:-$HOME/.claude}/skills
      claude-code -> same as cc
      dir         -> requires --dest
  --agent <name>
      Alias for --target.
  --dest <dir>
      Explicit destination directory. Overrides --target resolution.
  --version <latest|main|vX.Y.Z|X.Y.Z>
      Release or branch to install from. Default: latest.
  --repo <owner/repo>
      GitHub repository. Default: MosRat/GrokSearch-rs.
  --source <path>
      Install from a local skills directory, repository root, or .tar.gz archive.
  --no-overwrite
      Skip existing skill directories instead of replacing them.
  --dry-run
      Print what would be installed without writing files.
  -h, --help
      Show this help.

Examples:
  # Install to the current directory ./skills
  curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash

  # Install to Codex
  curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash -s -- --target codex

  # Install to Claude Code
  curl -fsSL https://raw.githubusercontent.com/MosRat/GrokSearch-rs/main/scripts/install-skills.sh | bash -s -- --target cc
EOF
}

die() {
  printf 'grok-search-rs skills installer: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --target|--agent)
      [ "$#" -ge 2 ] || die "$1 requires a value"
      TARGET="$2"
      shift 2
      ;;
    --dest)
      [ "$#" -ge 2 ] || die "$1 requires a value"
      DEST="$2"
      shift 2
      ;;
    --version)
      [ "$#" -ge 2 ] || die "$1 requires a value"
      VERSION="$2"
      shift 2
      ;;
    --repo)
      [ "$#" -ge 2 ] || die "$1 requires a value"
      REPO="$2"
      shift 2
      ;;
    --source)
      [ "$#" -ge 2 ] || die "$1 requires a value"
      SOURCE="$2"
      shift 2
      ;;
    --no-overwrite)
      OVERWRITE=0
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

resolve_dest() {
  if [ -n "$DEST" ]; then
    printf '%s\n' "$DEST"
    return
  fi

  case "$TARGET" in
    current|local|cwd)
      printf '%s\n' "$PWD/skills"
      ;;
    codex)
      if [ -n "${CODEX_HOME:-}" ]; then
        printf '%s\n' "$CODEX_HOME/skills"
      elif [ -n "${HOME:-}" ]; then
        printf '%s\n' "$HOME/.codex/skills"
      else
        die "HOME is unset; pass --dest for codex install"
      fi
      ;;
    cc|claude|claude-code|claude_code)
      if [ -n "${CLAUDE_HOME:-}" ]; then
        printf '%s\n' "$CLAUDE_HOME/skills"
      elif [ -n "${HOME:-}" ]; then
        printf '%s\n' "$HOME/.claude/skills"
      else
        die "HOME is unset; pass --dest for Claude Code install"
      fi
      ;;
    dir)
      die "--target dir requires --dest <dir>"
      ;;
    *)
      die "unsupported target: $TARGET"
      ;;
  esac
}

find_skills_root() {
  root="$1"
  if [ -d "$root/skills" ]; then
    printf '%s\n' "$root/skills"
    return
  fi
  if [ -f "$root/SKILL.md" ]; then
    printf '%s\n' "$(dirname "$root")"
    return
  fi
  found="$(find "$root" -maxdepth 3 -type d -name skills 2>/dev/null | head -n 1 || true)"
  [ -n "$found" ] || die "could not find a skills directory in $root"
  printf '%s\n' "$found"
}

TMPDIR_INSTALL=""
cleanup() {
  if [ -n "$TMPDIR_INSTALL" ] && [ -d "$TMPDIR_INSTALL" ]; then
    rm -rf "$TMPDIR_INSTALL"
  fi
}
trap cleanup EXIT INT TERM

prepare_source() {
  if [ -n "$SOURCE" ]; then
    if [ -d "$SOURCE" ]; then
      find_skills_root "$SOURCE"
      return
    fi
    [ -f "$SOURCE" ] || die "source does not exist: $SOURCE"
    need_cmd tar
    TMPDIR_INSTALL="$(mktemp -d)"
    tar -xzf "$SOURCE" -C "$TMPDIR_INSTALL"
    find_skills_root "$TMPDIR_INSTALL"
    return
  fi

  need_cmd curl
  need_cmd tar
  TMPDIR_INSTALL="$(mktemp -d)"
  archive="$TMPDIR_INSTALL/grok-search-rs-skills.tar.gz"

  case "$VERSION" in
    latest)
      url="https://github.com/$REPO/releases/latest/download/grok-search-rs-skills.tar.gz"
      ;;
    main|master)
      url="https://github.com/$REPO/archive/refs/heads/$VERSION.tar.gz"
      ;;
    v*)
      url="https://github.com/$REPO/releases/download/$VERSION/grok-search-rs-skills.tar.gz"
      ;;
    *)
      url="https://github.com/$REPO/releases/download/v$VERSION/grok-search-rs-skills.tar.gz"
      ;;
  esac

  printf 'Downloading %s\n' "$url" >&2
  curl -fsSL "$url" -o "$archive"
  tar -xzf "$archive" -C "$TMPDIR_INSTALL"
  find_skills_root "$TMPDIR_INSTALL"
}

DEST_DIR="$(resolve_dest)"
SKILLS_ROOT="$(prepare_source)"

[ -d "$SKILLS_ROOT" ] || die "skills root is not a directory: $SKILLS_ROOT"

count=0
skipped=0

if [ "$DRY_RUN" -eq 0 ]; then
  mkdir -p "$DEST_DIR"
fi

for skill_dir in "$SKILLS_ROOT"/*; do
  [ -d "$skill_dir" ] || continue
  [ -f "$skill_dir/SKILL.md" ] || continue
  name="$(basename "$skill_dir")"
  case "$name" in
    ""|*[!a-z0-9-]*)
      die "refusing suspicious skill directory name: $name"
      ;;
  esac
  target_dir="$DEST_DIR/$name"

  if [ -e "$target_dir" ] && [ "$OVERWRITE" -eq 0 ]; then
    printf 'Skipping existing %s\n' "$target_dir"
    skipped=$((skipped + 1))
    continue
  fi

  if [ "$DRY_RUN" -eq 1 ]; then
    printf 'Would install %s -> %s\n' "$name" "$target_dir"
  else
    stage="$DEST_DIR/.${name}.install.$$"
    rm -rf "$stage"
    cp -R "$skill_dir" "$stage"
    rm -rf "$target_dir"
    mv "$stage" "$target_dir"
    printf 'Installed %s -> %s\n' "$name" "$target_dir"
  fi
  count=$((count + 1))
done

[ "$count" -gt 0 ] || die "no skill directories with SKILL.md found in $SKILLS_ROOT"

if [ "$DRY_RUN" -eq 1 ]; then
  printf 'Dry run complete: %s skills would be installed to %s (%s skipped).\n' "$count" "$DEST_DIR" "$skipped"
else
  printf 'Done: installed %s GrokSearch-rs skills to %s (%s skipped).\n' "$count" "$DEST_DIR" "$skipped"
fi
