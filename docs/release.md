# Release Guide

## crates.io

This repository includes workflows for publishing:

- `context69-contracts`
- `context69-sdk`

Tag conventions:

- `contracts-v*`
- `sdk-v*`

## GHCR

This repository includes `.github/workflows/publish-docker-ghcr.yml`.

It publishes Docker images to:

```text
ghcr.io/<owner>/<repo>
```

Behavior:

- push to `main` publishes the `main` tag
- push of `v*` publishes the matching release tag
- default branch also publishes `latest`
- separate native `amd64` and `arm64` builds are merged into a multi-arch manifest
