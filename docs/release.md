# Release Guide

## Distribution model: git tags only

- No new versions are published to crates.io after `0.15.19`. The registry and
  docs.rs stay at `0.15.19` (not yanked); consume new releases from git tags
  and read the docs in this repository.
- `context69-contracts` and `context69-sdk` follow the workspace product
  version (`version.workspace = true`) and set `publish = false` as a guardrail
  against accidental registry publishes.
- All internal sub-crates stay at `version = "0.1.0"` with `publish = false`.
  They are never bumped, published, or tagged on their own.
- Tags are product-level only (`v*`, the `v` prefix plus the workspace product
  version). Sub-crate changes ride the next product tag.

## GitHub releases via release.yml

Workflow: `.github/workflows/release.yml`.

- Triggers: push of `v*` tags, plus `workflow_dispatch` with a required `tag`
  input to backfill a Release for an existing tag (for example `v0.16.0`).
- The resolve-tag step rejects non-`v*` values, then the workflow checks out
  that tag ref.
- The release job creates a GitHub Release for that tag with generated notes
  (`softprops/action-gh-release@v3`, `generate_release_notes: true`).
- Permissions are `contents: write`. Concurrency group is
  `release-${{ github.ref }}` with `cancel-in-progress: false`.
- Pushing `vX.Y.Z` only creates the GitHub Release; it never touches crates.io.

## SDK consumption

Add the SDK as a git dependency pinned to a product tag (same snippet as
`crates/context69-sdk/README.md`):

```toml
[dependencies]
context69-sdk = { git = "https://github.com/cloudiful/context69.git", tag = "v0.16.0" }
```

Pin a tag; path dependencies inside the checkout stay consistent automatically.

## CI build cache

The remaining Docker workflow (`.github/workflows/publish-docker-ghcr.yml`)
uses `sccache` with the GitHub Actions cache backend
(`SCCACHE_GHA_ENABLED: "true"`); it needs no extra credentials. No remaining
workflow uses the former R2-backed cache, so the old `R2_ACCESS_KEY_ID` /
`R2_SECRET_ACCESS_KEY` setup no longer applies.

## GHCR

Docker images are published by `.github/workflows/publish-docker-ghcr.yml`,
which also triggers on `v*` tags (plus manual dispatch).

- Image tags are derived from the git ref via `docker/metadata-action`.
- Native `amd64` and `arm64` runners build the backend binary and the frontend
  `dist`, then a runtime-only assembly step builds from those artifacts using
  the root `Dockerfile` (base `debian:trixie-slim`; Forgejo CI uses
  `forgejo.Dockerfile` with the internal mirror).
- The per-arch pushes are merged into a multi-arch manifest.
