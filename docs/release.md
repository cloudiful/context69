# Release Guide

## crates.io

This repository includes workflows for publishing:

- `context69-contracts`
- `context69-sdk`

Tag convention:

- `v*`

Behavior:

- pushing `v1.2.3` publishes both crates
- the workflow validates the `v*` tag format and publishes the versions declared in the workspace manifests
- after both crates publish successfully, the workflow creates a GitHub Release with generated release notes

## CI build cache

The Docker and crates.io workflows use `sccache` with the Cloudflare R2 S3-compatible backend.
Configure these repository secrets before running either workflow:

- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`

The workflows use the R2 endpoint from the reference deployment, bucket `sccache`, and the
shared `rust/` key prefix. Create the R2 API token with Object Read and Object Write access
limited to that bucket. The credentials are injected only into Rust build and cache-stat steps;
they are not passed to Docker image assembly or published as artifacts.

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
- Forgejo CI uses a separate `forgejo.Dockerfile` with Cloud1ful's Debian image and APT mirror
- separate native `amd64` and `arm64` image publishes are merged into a multi-arch manifest
