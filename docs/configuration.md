# Configuration

## Config File Location

`context69` reads its main config file from the platform config directory:

- Linux: `~/.config/context69/config.toml`
- macOS: `~/Library/Application Support/context69/config.toml`
- Windows: `%APPDATA%\context69\config.toml`

## Main Sections

- `app_db`: Context69 metadata database
- `qdrant`: runtime vector store defaults or bootstrap values
- `embedding`: runtime embedding defaults or bootstrap values
- `docling`: optional bootstrap values for file parsing
- `scheduler`: sync scheduling defaults
- `mcp`: MCP server configuration
- `api`: HTTP API server configuration
- `connections[]`: bootstrap source connections imported into the app database on first startup
- `sources[]`: bootstrap source definitions imported into the app database on first startup

At runtime, the database is the source of truth for:

- runtime settings
- docling settings
- source connections
- source definitions

That means the backend can start with only `app_db.url`. When runtime settings are still
empty or invalid, Context69 boots in degraded mode so the frontend settings page can be used
to finish configuration. Search and library ingest require a restart after those settings are saved.

## Scheduler Options

- `valkey_url`: optional; enables persistent scheduler state and distributed execution leasing
- `execution_guard_ttl_secs`: lease TTL, default `30`
- `execution_guard_renew_interval_secs`: lease renewal interval, default `10`

The renew interval must be lower than the TTL.

## Qdrant Options

- `recreate_on_dimension_mismatch`: when enabled, Context69 recreates the collection if the configured embedding dimension no longer matches the existing collection schema

## Docling Options

When `docling` is configured, Context69 uses:

- `cloudiful-docling-convert` for PDF and DOCX ingest
- Docling async JSON conversion for XLSX ingest

The supported config shape is:

- `docling.connection.base_url`
- `docling.connection.timeout_secs`
- `docling.connection.poll_interval_secs`
- `docling.connection.task_timeout_secs` (default `600`), the maximum time to wait for an async XLSX task after submission
- `docling.vlm.openai_base_url`
- `docling.vlm.api_key`
- `docling.vlm.vlm_pipeline_model`
- `docling.vlm.picture_description_model`
- `docling.vlm.code_formula_model`

The VLM block is optional. Leave all `docling.vlm.*` fields unset to disable VLM enrichment.
If you use raw Docling VLM config, set all five raw VLM fields together:

- `docling.vlm.openai_base_url`
- `docling.vlm.api_key`
- `docling.vlm.vlm_pipeline_model`
- `docling.vlm.picture_description_model`
- `docling.vlm.code_formula_model`

In the frontend settings page, configure Docling VLM directly with `openai_base_url`, `api_key`,
and the three model fields.

XLSX status and result requests retry transient connection errors, HTTP `429`, and `5xx`
responses with backoff. The async submission is not replayed, because the service may have
accepted it even when the response is lost.

Legacy OCR, PDF backend, image export, and enrichment toggle fields are no longer used.

## Secrets and Environment Overrides

Do not store production secrets in config files committed to source control.

Useful environment overrides:

- `CONTEXT69_APP_DB__URL`
- `CONTEXT69_SCHEDULER__VALKEY_URL`
- `CONTEXT69_FILE_LIBRARY__TRUSTED_PROXY_ENABLED`

Any nested config field can be overridden with `__` separators.

Example:

```bash
export CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69'
cargo run
```

If you prefer bootstrap-by-config instead of using the frontend, runtime-related overrides
such as `CONTEXT69_QDRANT__URL`, `CONTEXT69_EMBEDDING__API_KEY`, or Docling fields still work
and will be imported into the database on first startup.

## Trusted URL Import Proxy

URL imports ignore proxy environment variables by default. Enable
`file_library.trusted_proxy_enabled` from Runtime Settings, or use
`CONTEXT69_FILE_LIBRARY__TRUSTED_PROXY_ENABLED=true` during initial bootstrap, to trust the
deployment's HTTPS egress proxy. The setting takes effect for new downloads immediately.

When enabled, Context69 uses `HTTPS_PROXY`/`https_proxy`, falling back to
`ALL_PROXY`/`all_proxy`, and applies `NO_PROXY`/`no_proxy`. The proxy endpoint is resolved and
validated separately from the requested public URL. The egress proxy must enforce destination
network isolation because it performs the final target connection.

## SQLx CLI

Root `sqlx.toml` makes SQLx macros and `cargo sqlx prepare` read
`CONTEXT69_APP_DB__URL` by default.
`cargo run --bin db_init -- --database-url ...` has highest priority.
Without that flag, `db_init` loads root `.env` if present, then resolves
`CONTEXT69_APP_DB__URL`, `DATABASE_URL`, and finally `app_db.url`.

For local development, set `CONTEXT69_APP_DB__URL` as the canonical value.
Keep `DATABASE_URL` aligned only if you still use tools that expect it:

```bash
export CONTEXT69_APP_DB__URL='postgres://postgres:postgres@127.0.0.1:5432/context69'
export DATABASE_URL="$CONTEXT69_APP_DB__URL"
cargo run --bin db_init
cargo sqlx prepare --workspace -- --all-targets
```
