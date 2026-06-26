# Configuration

## Config File Location

`context69` reads its main config file from the platform config directory:

- Linux: `~/.config/context69/config.toml`
- macOS: `~/Library/Application Support/context69/config.toml`
- Windows: `%APPDATA%\context69\config.toml`

## Main Sections

- `connections[]`: external source database connections
- `sources[]`: source definitions used as import templates before source records exist in the app database
- `app_db`: Context69 metadata database
- `qdrant`: vector store connection
- `embedding`: embedding provider configuration
- `docling`: optional file parsing service configuration
- `scheduler`: sync scheduling configuration
- `mcp`: MCP server configuration
- `api`: HTTP API server configuration

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
- `docling.vlm.openai_base_url`
- `docling.vlm.api_key`
- `docling.vlm.vlm_pipeline_model`
- `docling.vlm.picture_description_model`
- `docling.vlm.code_formula_model`

Legacy OCR, PDF backend, image export, and enrichment toggle fields are no longer used.

## Secrets and Environment Overrides

Do not store production secrets in config files committed to source control.

Useful environment overrides:

- `CONTEXT69_APP_DB__URL`
- `CONTEXT69_QDRANT__URL`
- `CONTEXT69_EMBEDDING__API_KEY`
- `CONTEXT69_DOCLING__VLM__API_KEY`
- `CONTEXT69_SCHEDULER__VALKEY_URL`

Any nested config field can be overridden with `__` separators.

Example:

```bash
export CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69'
export CONTEXT69_EMBEDDING__API_KEY='sk-xxx'
cargo run
```

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
