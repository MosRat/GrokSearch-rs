@C:\Users\taizun\.codex\RTK.md

# GrokSearch-rs Agent Notes

## Release and Version Bump Rules

- Normal releases must use the GitHub Actions orchestration flow documented in
  `RELEASING.md`.
- Do not bump release manifests locally for a normal release. In particular, do
  not manually edit the version in `Cargo.toml`, `Cargo.lock`,
  `npm/grok-search-rs/package.json`, `npm/platforms/*/package.json`,
  `python/pyproject.toml`, or `python/grok_search_rs/__init__.py` just to cut a
  release.
- Do not create release tags locally by hand. Tags from the manually dispatched
  bump workflow do not reliably trigger the tag-push release workflow, so the
  repository uses an explicit second workflow dispatch.

### Normal Release Flow

1. Ensure code changes are committed and pushed to `main`.
2. Add or update the target version section in `CHANGELOG.md`, commit it, and
   push it before publishing.
3. Run the orchestrating script from PowerShell:

   ```powershell
   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-via-workflows.ps1 0.1.27
   ```

4. The script dispatches `.github/workflows/bump.yml` on `main` with
   `dry_run=false`.
5. `bump.yml` updates Cargo, npm, and Python package metadata, commits the
   manifest bump, and pushes tag `vX.Y.Z`.
6. The script waits for the tag, then dispatches `.github/workflows/release.yml`
   on `vX.Y.Z`.
7. `release.yml` builds cross-platform assets, creates the GitHub Release,
   publishes npm packages, publishes PyPI wheels, and syncs release metadata.

### Manual Fallback

- If the orchestration script cannot run, use the GitHub UI:
  Actions -> Bump Version -> Run workflow, wait for `vX.Y.Z`, then
  Actions -> Release -> Run workflow on the new tag.
- Use `scripts/bump-version.sh` only for offline emergency fallback. It is not
  the normal release path.

### Verification

- Confirm the GitHub release page has all archives plus `SHA256SUMS`.
- Confirm `main` has the Actions-created manifest bump commit.
- Confirm:

  ```powershell
  uv tool install grok-search-rs==X.Y.Z
  uvx grok-search-rs@X.Y.Z --version
  npx grok-search-rs@X.Y.Z --help
  ```
