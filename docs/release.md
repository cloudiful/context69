# Release Guide

## crates.io

This repository includes workflows for publishing:

- `context69-contracts`
- `context69-sdk`

Tag convention:

- `v*`

Behavior:

- pushing `v1.2.3` publishes both crates
- both crate versions must match `1.2.3`, or the workflow fails before publish

## GHCR

This repository includes `.github/workflows/publish-docker-ghcr.yml`.

It publishes Docker images to:

```text
ghcr.io/<owner>/<repo>
```

Behavior:

- push to `main` publishes the `main` tag
- push of `v*` publishes the matching release tag
- push of `v*` also publishes both crates, so Docker and crates share one release tag
- default branch also publishes `latest`
- native `amd64` and `arm64` runners build the backend binary and frontend `dist` as per-arch artifacts
- a runtime-only Docker assembly job builds from those artifacts using the root `Dockerfile`
- the runtime image base is public `debian:trixie-slim`
- Forgejo CI uses a separate `Dockerfile.forgejo` with Cloud1ful's Debian image and APT mirror
- separate native `amd64` and `arm64` image publishes are merged into a multi-arch manifest
