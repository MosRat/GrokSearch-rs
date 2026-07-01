# Releasing grok-search-rs

## The only thing you do

```bash
# 1. (optional) add a section to CHANGELOG.md and push it
$EDITOR CHANGELOG.md
git commit -am "docs: changelog for 0.1.5"
git push

# 2. Let GitHub Actions bump, commit, and tag; then dispatch release for that tag
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-via-workflows.ps1 0.1.5
```

That's all. The script triggers `bump.yml`, waits for it to commit the version
bump and push `v0.1.5`, then manually triggers `release.yml` on that tag.

Do not create release tags locally by hand. Tags created by the manually
dispatched bump workflow do not reliably trigger the tag-push release workflow,
so the release dispatch is an explicit second step in the script.

The release workflow then:

1. Injects `0.1.5` into `Cargo.toml` in the CI working tree and builds cross-platform binaries.
   Linux assets are static musl binaries built with Zig and `cargo zigbuild`.
2. Creates the GitHub Release with platform archives, the skills archive, and
   `SHA256SUMS`.
3. Publishes the 6 npm packages: main package plus 5 platform packages.
4. Publishes platform wheels to PyPI so `uv tool install grok-search-rs` and `uvx grok-search-rs` work.
5. Commits the version bump back to `main` so Cargo, npm, and Python package metadata stay in sync.

## Manual fallback

If the orchestrating script is unavailable:

- GitHub UI: Actions -> Bump Version -> Run workflow
- Wait for the `vX.Y.Z` tag from the bump workflow
- GitHub UI: Actions -> Release -> Run workflow -> select the new tag

If CI is unavailable and you truly need to bump manifests locally:

- Local script: `scripts/bump-version.sh 0.1.5 --push`

The local script predates the workflow-orchestrated release path and remains
for offline use only.

## Where version numbers live

- `Cargo.toml` - auto-synced to `main` by the `sync-main` job
- `Cargo.lock` - refreshed alongside `Cargo.toml`
- `npm/grok-search-rs/package.json` - auto-synced
- `npm/platforms/*/package.json` - auto-synced
- `python/pyproject.toml` - auto-synced
- `python/grok_search_rs/__init__.py` - auto-synced

## Prerequisites

- `secrets.NPM_TOKEN` configured
- `secrets.PYPI_API_TOKEN` configured from a PyPI API token
- No branch protection rule on `main` blocking `github-actions[bot]`
- CI Linux release builds install Zig and `cargo-zigbuild`; local Linux release
  verification needs both on `PATH`.

## Verify after release

- GitHub release page lists 5 platform archives, `grok-search-rs-skills.tar.gz`,
  and `SHA256SUMS`
- Linux archives contain musl binaries from `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl`
- `npx grok-search-rs@X.Y.Z --help` works
- `uv tool install grok-search-rs==X.Y.Z` works
- `uvx grok-search-rs@X.Y.Z --version` works
- `main` has a `chore: sync manifests to X.Y.Z` commit from `github-actions[bot]`
