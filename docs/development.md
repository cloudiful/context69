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

One one-shot CLI mode maintains library storage. It does not delete old objects
automatically.

`migrate-library-storage [--dry-run]` copies files from the local filesystem
storage root into the active S3 backend. It requires S3 to be configured.

```bash
cargo run -- migrate-library-storage --dry-run
```

### Legacy UUID direct-path migration (automatic)

Legacy library files that still point at UUID direct paths
(`storage_object_id IS NULL`) are migrated automatically at application
startup, before pending task workers resume. The migration runs on every normal
startup: it reads each source object back through the storage abstraction,
verifies it against the stored size and SHA-256, links the row to the existing
content-addressed layout (`objects/{group_id}/{sha256}`), and records the old
key durably in `context69.library_legacy_object_cleanup` for a separate, later
cleanup phase. The operation is bounded per selection page (default batch size
100), idempotent, and restartable; per-row missing/invalid/errors are logged and
retried on the next startup. A fatal migration selection error is logged and
tolerated so unrelated startup behavior is unaffected, and the migration is
retried on the next restart. Old physical objects are never deleted by this
phase.

The startup log line exposes the run summary:
`scanned`, `migrated`, `already_migrated`, `missing`, `invalid`, `conflicts`, and
`errors`.

### Old-object and code cleanup (awaiting explicit confirmation)

Old physical object deletion and removal of the legacy read-compatibility code are
deliberately deferred to a separately reviewed phase that runs only after the
user confirms the startup migration is complete. Do not run a manual cleanup
against production from this workspace, and do not remove the legacy
compatibility code until that confirmation is given.

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
