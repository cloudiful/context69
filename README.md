# context69

`context69` is a retrieval-focused knowledge base service for ingesting text from external sources, indexing it into Qdrant, and serving both HTTP API and MCP query interfaces.

## What It Does

- Pulls text records from PostgreSQL-backed sources
- Normalizes, chunks, and embeds documents
- Stores vectors in Qdrant
- Exposes a search API and MCP endpoints
- Includes an optional Vue-based web UI

## Current Scope

- Retrieval only, no answer-generation layer
- Built-in source connector: `postgres_sql`
- HTTP API, MCP over HTTP, and MCP over stdio

## Quick Start

### Prerequisites

- Rust toolchain
- PostgreSQL for application metadata
- Qdrant
- An OpenAI-compatible embedding API

### Run the service

```bash
cargo run
```

By default this starts:

- HTTP API
- Scheduler
- MCP HTTP server when enabled in config

### One-off commands

Export OpenAPI:

```bash
cargo run -- export-openapi
```

Run a single sync:

```bash
cargo run -- sync-once
```

Run MCP over stdio:

```bash
cargo run -- mcp-stdio
```

## Configuration

The application reads configuration from the platform config directory:

- Linux: `~/.config/context69/config.toml`
- macOS: `~/Library/Application Support/context69/config.toml`
- Windows: `%APPDATA%\context69\config.toml`

For local setup, start from:

- [`.env.example`](/Users/cloudiful/codes/research/context69/.env.example:1)

Common environment overrides:

- `CONTEXT69_APP_DB__URL`
- `CONTEXT69_QDRANT__URL`
- `CONTEXT69_EMBEDDING__API_KEY`
- `CONTEXT69_DOCLING__VLM__API_KEY`
- `CONTEXT69_SCHEDULER__VALKEY_URL`

Detailed configuration docs:

- [Configuration](/Users/cloudiful/codes/research/context69/docs/configuration.md)
- [PostgreSQL Source Requirements](/Users/cloudiful/codes/research/context69/docs/postgres-source.md)
- [MCP](/Users/cloudiful/codes/research/context69/docs/mcp.md)

## Docker

Build the all-in-one image:

```bash
docker build -t context69:latest .
```

Run it:

```bash
docker run --rm \
  -p 80:80 \
  -e CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69' \
  -e CONTEXT69_EMBEDDING__API_KEY='sk-xxx' \
  context69:latest
```

More details:

- [Docker Deployment](/Users/cloudiful/codes/research/context69/docs/docker.md)

## GitHub Actions and GHCR

This repository includes GitHub Actions workflows for:

- publishing crates to crates.io
- building and publishing Docker images to `ghcr.io`

The Docker workflow uses native GitHub-hosted runners for both `amd64` and `arm64`, then publishes a multi-architecture image manifest.

Release details:

- [Release Guide](/Users/cloudiful/codes/research/context69/docs/release.md)

## Development

The repository contains a Vue 3 + Vite + bun frontend under `frontend/`.

Development guides:

- [Development Guide](/Users/cloudiful/codes/research/context69/docs/development.md)
- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- [Architecture Notes](/Users/cloudiful/codes/research/context69/docs/architecture.md)

## API

Runtime endpoints include:

- `GET /healthz`
- `GET /openapi.json`
- `POST /v1/search`
- source and document management endpoints under `/v1/*`

For the full surface:

- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- generated OpenAPI output at `frontend/openapi/context69.openapi.json`

## Security

- Do not commit real credentials, API keys, or database URLs
- Use environment variables or your platform secret manager
- Keep `.env` files local only

## License

Apache-2.0
