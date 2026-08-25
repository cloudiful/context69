# Development Guide

## Frontend

The frontend lives in `frontend/` and uses:

- Vue 3
- Vite
- bun

Install dependencies:

```bash
mise install
cd frontend
bun install
```

Generate frontend API types from OpenAPI:

```bash
cd frontend
bun run generate:api
```

Or let the local full-stack launcher refresh them for you:

```bash
nu scripts/dev.nu full
```

Start the frontend dev server:

```bash
cd frontend
bun run dev
```

By default it runs at:

```text
http://127.0.0.1:5173
```

and proxies `/healthz` and `/v1/*` to `http://127.0.0.1:8096`.

## Backend

Browser sessions require Valkey. They reuse the scheduler Valkey URL saved in runtime Settings; when it is unset, the local default is `redis://127.0.0.1:6379`. The cookie signing key is generated once and stored internally in PostgreSQL. An environment override remains available for recovery when a saved Valkey URL is unavailable:

```bash
export CONTEXT69_AUTH__SESSION_VALKEY_URL=redis://127.0.0.1:6382
```

`CONTEXT69_AUTH__SESSION_SECRET_KEY` is an optional break-glass override and must contain at least 32 characters when set. All instances must use the same PostgreSQL database and Valkey; do not set different secret overrides per instance.

Run the backend:

```bash
cargo run
```

Or use the local dev launcher:

```bash
nu scripts/dev.nu backend
```

## Library Storage Maintenance Modes

Two one-shot CLI modes maintain library storage. Neither deletes old objects
automatically.

`migrate-library-storage [--dry-run]` copies files from the local filesystem
storage root into the active S3 backend. It requires S3 to be configured.

```bash
cargo run -- migrate-library-storage --dry-run
```

`migrate-library-legacy-paths [--dry-run] [--batch-size <n>]` migrates legacy
library files that still point at UUID direct paths
(`storage_object_id IS NULL`) onto the content-addressed layout
(`objects/{group_id}/{sha256}`). Each source object is read back and verified
against its stored size and SHA-256 before the reference is updated; the old
key is recorded in `context69.library_legacy_object_cleanup` for a separate,
later cleanup phase and is never deleted by this command. The run is bounded
(default batch size 100), restartable, and idempotent; per-row failures are
counted and reported without blocking later rows.

```bash
cargo run -- migrate-library-legacy-paths --dry-run
cargo run -- migrate-library-legacy-paths --batch-size 200
```

`cleanup-library-legacy-paths [--dry-run] [--execute] [--batch-size <n>]`
deletes the physical old objects recorded in
`context69.library_legacy_object_cleanup` once their grace period has
elapsed. Migrations 0023 and 0024 must be applied before running it. The
mode defaults to a dry run; `--execute` is required for actual deletion and
`--dry-run` and `--execute` cannot be combined. A record is skipped (and
left open) when its old key is still referenced by any
`library_files.storage_rel_path` row, when it was recorded on a storage
backend different from the active one, or when its recorded backend is
unknown (pre-0024 rows have no backend recorded and are never deleted).
Physical deletion happens first; only then is the record marked deleted,
so an interrupted run can be restarted safely and already-missing objects
count as idempotent successes.

```bash
cargo run -- cleanup-library-legacy-paths --dry-run
cargo run -- cleanup-library-legacy-paths --dry-run --batch-size 200
cargo run -- cleanup-library-legacy-paths --execute --batch-size 200
```

## Local Full-Stack Flow

Preferred single-command flow:

```bash
nu scripts/dev.nu full
```

What it does:

1. Builds `context69`
2. Exports `frontend/openapi/context69.openapi.json`
3. Regenerates `frontend/src/generated/openapi.ts`
4. Starts the backend and waits for `/healthz` and `/mcp`
5. Starts the frontend Vite server at `http://127.0.0.1:5173`

Manual flow if you want separate terminals:

1. Start the backend with `cargo run`
2. Start the frontend with `bun run dev`
3. Regenerate API types when the OpenAPI contract changes
