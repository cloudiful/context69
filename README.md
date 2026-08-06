# context69

[![Docker Image Workflow](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml)
[![Crates Workflow](https://github.com/cloudiful/context69/actions/workflows/publish-crates.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-crates.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/cloudiful/context69?sort=semver)](https://github.com/cloudiful/context69/releases)

[中文文档](README.zh-CN.md)

Context69 is a retrieval-focused knowledge base service for ingesting documents, creating vector indexes, and exposing search through HTTP APIs and MCP.

## Features

- Ingest text, URLs, and library files from PostgreSQL-backed sources
- Normalize, chunk, embed, and store vectors in Qdrant
- Support PDF, DOCX, and XLSX conversion through Docling
- Provide multilingual translation and locale-aware retrieval
- Auto-clean task history with an admin maintenance panel (retention window, cancel-active, purge)
- Expose an HTTP API, MCP over HTTP or stdio, and an optional web UI

## Deployment

### Docker image

The published image is available from GHCR. Replace `<tag>` with a release tag such as `v0.8.1`.

Requirements:

- PostgreSQL for application data
- Qdrant and an embedding provider for search and ingestion
- Valkey is optional for shared sessions and scheduler coordination
- Docling is optional and required for PDF, DOCX, or XLSX conversion

```bash
docker volume create context69-library

docker run -d \
  --name context69 \
  --restart unless-stopped \
  -p 80:80 \
  -p 8097:8097 \
  -v context69-library:/app/data/library \
  -e CONTEXT69_APP_DB__URL='postgres://user:password@db/context69' \
  ghcr.io/cloudiful/context69:<tag>
```

Open `http://localhost` after the first start. Configure Qdrant, the embedding provider, and any optional services in the Settings page, then restart the container. Port `80` serves the web UI and HTTP API; port `8097` serves MCP over HTTP.

### Build from source

```bash
cargo build --release --bin context69
cd frontend
bun install --no-save
bun run build
cd ..
mkdir -p ci-image-input/frontend-dist
install -Dm755 target/release/context69 ci-image-input/context69
cp -R frontend/dist/. ci-image-input/frontend-dist/
docker build -t context69:latest .
```

Run the locally built image with the Docker command above and replace the image name with `context69:latest`.

## License

Apache-2.0. See [LICENSE](LICENSE).
