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

## Structured Document Store

Context69 stores normalized document text and arbitrary JSON metadata. Source systems remain
responsible for vendor payloads and original HTML/PDF/image retention; `source_uri` links back to
those objects.

Document identity is `(group, source_key, external_id)`. Group-scoped APIs support exact and batch
lookup, deletion, filtered cursor pagination, and up to three sort fields. `published_at`,
`published_after`, and `published_before` use RFC 3339 timestamps. Migrated dates use UTC midnight
unless `metadata_json.published_at` contained a precise RFC 3339 value.

Metadata remains schema-free for writes. Filtering or sorting requires a group/source metadata
index declaration. Dot paths such as `provider.name` are supported. Indexes build in the
background and become queryable only in `ready` state. Scalars support `eq`, `in`, `range`, and
`exists`; arrays support `contains` and `exists`.

## Translation and multilingual search

Context69 keeps translations as rebuildable language variants linked to the canonical document.
The original title, summary, body, metadata, timestamps, and business identity are never replaced.
After original ingest succeeds, translation jobs run asynchronously and publish translated chunks
to PostgreSQL and Qdrant only after the whole document succeeds.

Group settings define default target locales and an optional source locale/glossary. Every text,
multipart, deduplicated prepare-upload, and URL import request can provide `translation`:

- omitted: inherit group defaults
- `{"target_locales":[]}`: disable translation for this ingest
- non-empty `target_locales`: override group defaults

Providers run in configured priority order. DeepL and LLM providers have independent UTC-month
character limits; LibreTranslate is unlimited when its limit is absent. A failed, rate-limited,
invalid, or quota-exhausted provider causes the whole document to restart on the next provider, so
one translation never mixes providers.

DeepL Free defaults to `https://api-free.deepl.com`; DeepL Pro defaults to
`https://api.deepl.com`. A custom compatible endpoint may still be configured explicitly.

Search, exact read, batch read, and structured document query accept an optional BCP 47 `locale`.
When a requested translation is unavailable, reads return the canonical original with
`is_fallback: true`, `content_locale`, and the current `translation_status`. Omitting `locale`
preserves the pre-0.7 original-only behavior.

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
If the configured browser-session Valkey is unavailable, the service still starts with an
in-memory session store so you can open Settings and correct the Valkey URL. Those sessions
are lost on restart; configure Valkey and restart the service to restore shared session storage.
Text-only ingest does not require Docling. PDF/DOCX/XLSX conversion needs Docling connection
settings, while Docling VLM fields can be left empty unless you want VLM-based enrichment.
Async XLSX conversion polls until a terminal task state, retries transient status/result failures,
and is bounded by the configured Docling task timeout (default 600 seconds).

The service records the active embedding Base URL, model, and dimensions for each Qdrant
collection. Changing any of them starts a background rebuild of all stored chunk vectors from
PostgreSQL during the next startup; health and settings stay available, while vector search returns
an unavailable error until the rebuild succeeds. Docling conversion and chunking are not repeated.
A missing collection is restored the same way. Administrators can also start an online rebuild and
monitor its progress from Runtime Settings. Do not stop the service while a rebuild is running.

File originals use `file_library.storage_root` by default. When the runtime file-library S3
settings are complete, the service uses the configured S3-compatible bucket instead. All service
instances must use the same S3 settings when sharing a database. S3 credentials are never returned
by the settings API; leaving Secret Key empty preserves the stored value.

Remote URL imports connect directly by default and ignore proxy environment variables. Enable
`file_library.trusted_proxy_enabled` in Runtime Settings to use `HTTPS_PROXY`/`https_proxy`, with
`ALL_PROXY`/`all_proxy` as fallback and `NO_PROXY`/`no_proxy` exclusions. The change applies to new
downloads immediately. The configured proxy must enforce destination network isolation because it
resolves and connects to the final target on Context69's behalf.

By default this starts:

- HTTP API
- Scheduler when sync runtime is configured
- MCP HTTP server when enabled in config

### One-off commands

Export OpenAPI:

```bash
cargo run -- export-openapi
```

Migrate verified local originals to the configured S3 backend:

```bash
cargo run -- migrate-library-storage --dry-run
cargo run -- migrate-library-storage
```

The migration is resumable. Missing files and SHA-256 mismatches are reported and left unchanged.

Export OpenAPI and regenerate frontend types:

```bash
cd frontend
bun run generate:api:from-backend
```

Regenerate frontend API types and run the production build:

```bash
cd frontend
bun run build:with-api
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
- `CONTEXT69_FILE_LIBRARY__TRUSTED_PROXY_ENABLED`

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
bun install --no-save
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

The bundled Nginx proxy does not emit access or error logs; application logs remain available from
the container output.

Then open the frontend settings page and save runtime and Docling settings.
Until that happens, `/healthz` is expected to report a degraded state.

More details:

- [Docker Deployment](/Users/cloudiful/codes/research/context69/docs/docker.md)

## GitHub Actions and GHCR

This repository includes GitHub Actions workflows for:

- publishing crates to crates.io
- building and publishing Docker images to `ghcr.io`

The Docker workflow builds backend and frontend artifacts on native GitHub-hosted runners for both `amd64` and `arm64`, then assembles and publishes runtime-only images before publishing a multi-architecture manifest.
Release tags use `v*`; the same tag publishes Docker and both crates.

Forgejo also has a separate workflow that publishes a `linux/arm64` image for pushes to `main` and manual runs. It does not process release tags; formal releases and multi-architecture GHCR images remain managed by GitHub Actions.

Release details:

- [Release Guide](/Users/cloudiful/codes/research/context69/docs/release.md)

## Development

The repository contains a Vue 3 + Vite + bun frontend under `frontend/`.

The frontend uses Nuxt UI with Vue 3, Vite, Tailwind CSS, and bundled Lucide icons. Prefer Nuxt UI
components and default theme tokens before adding project-specific controls or CSS.

The frontend OpenAPI generator uses TypeScript 5.9.3 because `openapi-typescript` 7.x currently
requires the TypeScript 5 API. Vue SFC checking remains on the separate `typescript-vue-compat`
7.0.2 alias through the custom `bun run check` script; do not replace it with bare `tsc`, which does
not type-check `.vue` files.

Development guides:

- [Development Guide](/Users/cloudiful/codes/research/context69/docs/development.md)
- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- [Architecture Notes](/Users/cloudiful/codes/research/context69/docs/architecture.md)

Local dev entrypoints:

```bash
nu scripts/dev.nu backend
nu scripts/dev.nu full
```

The full development stack binds Vite to `0.0.0.0:5173`, prints the available LAN URLs on
startup, and proxies backend requests to `http://127.0.0.1:8096`.

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
- `GET /v1/library/resources` for database-backed folder pagination, search, and sorting in the file library
- `GET /v1/groups` and `GET /v1/groups/by-path/{group_path}/children` return paginated group pages
- `GET /v1/groups/by-path/{group_path}/members` and `GET /v1/admin/users` return paginated, searchable pages
- `GET /v1/groups/search` provides bounded remote group search for move targets
- `GET /v1/groups/by-path/{group_path}/library/resources` for database-backed folder pagination, search, and sorting
- `POST /v1/groups/by-path/{group_path}/library/files/{file_id}/retry` to reprocess a failed file from its saved original
- `POST /v1/groups/by-path/{group_path}/library/files/prepare-upload` to reuse an existing SHA-256 object inside the same group while applying optional business metadata
- `GET|POST /v1/auth/personal-access-tokens`
- `DELETE /v1/auth/personal-access-tokens/{token_id}`
- source and document management endpoints under `/v1/*`

For the full surface:

- [API Reference](/Users/cloudiful/codes/research/context69/docs/api.md)
- generated OpenAPI output at `frontend/openapi/context69.openapi.json`
- JSON library text endpoints accept `content_format = plain_text | markdown`; Markdown requests are stored as `.md` with `text/markdown`, while multipart uploads already support `.md` files directly
- Multipart file uploads accept an `application/json` `metadata` part immediately before its `files` part. It contains `external_id`, `source_uri`, RFC 3339 `published_at`, and object-valued `metadata_json`. Every parsed section inherits these fields.
- `POST /v1/groups/by-path/{group_path}/library/files/import-url` accepts a public HTTPS file URL and returns an asynchronous URL import job. Context69 validates every DNS/redirect target against private and special-use networks, limits downloads to `max_upload_size_mb`, persists the original in the configured S3/local content store, then reuses the normal SHA deduplication and ingest pipeline. URL imports do not send cookies, authorization, or custom headers and do not parse HTML landing pages. They connect directly unless the trusted environment proxy setting is enabled.
- URL import jobs are read at `GET .../library/url-import-jobs/{job_id}` and failed jobs can be retried with `POST .../{job_id}/retry`. Originals remain managed library files until deletion; ingest-only retries reuse the stored original without downloading it again.
- Text JSON upserts and binary uploads share the same internal metadata composition path: section metadata, then file business metadata, then protected Context69 library fields.
- A repeated `(group, external_id, SHA-256)` upload updates metadata without parsing or embedding again. A changed SHA-256 replaces and re-ingests that logical file. Reusing one SHA-256 with another external ID returns `409 external_id_content_conflict`.

## Security

- Browser authentication uses the signed `context69_session_v2` HttpOnly cookie and a shared Valkey session store when Valkey is available. Startup falls back to an in-memory store when Valkey is unavailable so Settings remains accessible for repair.
- Browser sessions reuse the runtime scheduler Valkey URL configured in Settings. If none is configured, the local default is `redis://127.0.0.1:6379`. Context69 generates the shared signing key once and stores it internally in PostgreSQL, so instances using the same database require no separate key configuration.
- Set `CONTEXT69_AUTH__SESSION_VALKEY_URL` or `CONTEXT69_AUTH__SESSION_SECRET_KEY` only as break-glass overrides. Production deployments should set `CONTEXT69_AUTH__SESSION_COOKIE_SECURE=true`.
- Browser UI and API must remain same-origin, either directly or through the documented frontend reverse proxy. Session cookies are not configured for cross-origin API access.
- Personal access tokens are user-scoped bearer credentials for CLI, MCP, or automation callers.
- Access token plaintext is returned only once at creation time; after that only metadata is listed in the UI and API.
- Personal access tokens always expire and can be revoked from the frontend settings page.
- Personal access tokens inherit the owning user's permissions and are further limited by the selected coarse scopes.
- Do not commit real credentials, API keys, or database URLs
- Use environment variables or your platform secret manager
- Keep `.env` files local only

## License

Apache-2.0
