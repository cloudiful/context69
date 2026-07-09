# context69

`context69` is a retrieval-focused knowledge base service for ingesting text from external sources, indexing it into Qdrant, and serving both HTTP API and MCP query interfaces.

## What It Does

- Pulls text records from PostgreSQL-backed sources
- Normalizes, chunks, and embeds documents
- Stores vectors in Qdrant
- Converts library PDF/DOCX files through `cloudiful-docling-convert`
- Converts library XLSX files through the Docling async JSON API
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
- Qdrant and an embedding provider only after you want search/ingest to become active

### Run the service

```bash
cargo run
```

With the current startup path, only `app_db.url` is required for the backend to boot.
If runtime settings are still empty, the service starts in degraded mode so you can open
the frontend and save Qdrant, embedding, Docling, scheduler, and source settings there.
Search and library ingest become available after those settings are saved and the service restarts.
Text-only ingest does not require Docling. PDF/DOCX/XLSX conversion needs Docling connection
settings, while Docling VLM fields can be left empty unless you want VLM-based enrichment.

By default this starts:

- HTTP API
- Scheduler when sync runtime is configured
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

Initialize the application database:

```bash
cargo run --bin db_init
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
- `CONTEXT69_SCHEDULER__VALKEY_URL`

Detailed configuration docs:

- [Configuration](/Users/cloudiful/codes/research/context69/docs/configuration.md)
- [PostgreSQL Source Requirements](/Users/cloudiful/codes/research/context69/docs/postgres-source.md)
- [MCP](/Users/cloudiful/codes/research/context69/docs/mcp.md)

## Docker

The root `Dockerfile` is now a runtime-only assembly image. It expects prebuilt inputs under
`ci-image-input/`:

- `ci-image-input/context69`
- `ci-image-input/frontend-dist/`

Build the backend and frontend first:

```bash
cargo build --release --bin context69
cd frontend
bun install --frozen-lockfile
bun run build
cd ..
mkdir -p ci-image-input/frontend-dist
install -Dm755 target/release/context69 ci-image-input/context69
cp -R frontend/dist/. ci-image-input/frontend-dist/
```

Then build the all-in-one image:

```bash
docker build -t context69:latest .
```

Run it:

```bash
docker run --rm \
  -p 80:80 \
  -e CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69' \
  context69:latest
```

Then open the frontend settings page and save runtime/provider/docling settings.
Until that happens, `/healthz` is expected to report a degraded state.

More details:

- [Docker Deployment](/Users/cloudiful/codes/research/context69/docs/docker.md)

## GitHub Actions and GHCR

This repository includes GitHub Actions workflows for:

- publishing crates to crates.io
- building and publishing Docker images to `ghcr.io`

The Docker workflow builds backend and frontend artifacts on native GitHub-hosted runners for both `amd64` and `arm64`, then assembles and publishes runtime-only images before publishing a multi-architecture manifest.
Release tags use `v*`; the same tag publishes Docker and both crates.

Release details:

- [Release Guide](/Users/cloudiful/codes/research/context69/docs/release.md)

## Development

The repository contains a Vue 3 + Vite + bun frontend under `frontend/`.

Development guides:

- [Development Guide](/Users/cloudiful/codes/research/context69/docs/development.md)
- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- [Architecture Notes](/Users/cloudiful/codes/research/context69/docs/architecture.md)

Local dev entrypoints:

```bash
nu scripts/dev.nu backend
nu scripts/dev.nu full
```

Database and SQLx workflow:

```bash
cargo run --bin db_init
cargo sqlx prepare --workspace -- --all-targets
```

Notes:

- `db_init` is migration-only. It accepts `--database-url` and otherwise loads root `.env` first when present, then resolves `CONTEXT69_APP_DB__URL`, `DATABASE_URL`, and finally `app_db.url`.
- SQLx now uses root `sqlx.toml`, so `cargo sqlx prepare` and `sqlx::migrate!()` follow the same `CONTEXT69_APP_DB__URL` variable and `migrations/` directory by default.
- Static runtime SQL should live under `src/sql/**` and be loaded with `sqlx::query_file*!` macros. Keep only genuinely dynamic SQL construction in Rust.
- SQLx metadata is stored in `.sqlx/` and should be refreshed after migration or checked-query changes.

## API

Runtime endpoints include:

- `GET /healthz`
- `GET /openapi.json`
- `POST /v1/search`
- `GET|POST /v1/auth/personal-access-tokens`
- `DELETE /v1/auth/personal-access-tokens/{token_id}`
- source and document management endpoints under `/v1/*`

For the full surface:

- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- generated OpenAPI output at `frontend/openapi/context69.openapi.json`

## Security

- Personal access tokens are user-scoped bearer credentials for CLI, MCP, or automation callers.
- Access token plaintext is returned only once at creation time; after that only metadata is listed in the UI and API.
- Personal access tokens always expire and can be revoked from the frontend settings page.
- Personal access tokens inherit the owning user's permissions and are further limited by the selected coarse scopes.
- Do not commit real credentials, API keys, or database URLs
- Use environment variables or your platform secret manager
- Keep `.env` files local only

## License

Apache-2.0
