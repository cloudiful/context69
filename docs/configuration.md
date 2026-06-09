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
