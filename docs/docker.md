# Docker Deployment

## All-in-One Image

The root `Dockerfile` is a runtime-only assembly image based on `debian:trixie-slim`. It does not
compile Rust or frontend assets inside Docker. Instead it copies prebuilt inputs from
`ci-image-input/`:

- `ci-image-input/context69`
- `ci-image-input/frontend-dist/`

Prepare those inputs first:

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

Build:

```bash
docker build -t context69:latest .
```

Forgejo CI uses `forgejo.Dockerfile` instead. That file uses
`dockerhub.cloud1ful.com/library/debian:trixie-slim` and `apt.cloud1ful.com`; keep it out of
GitHub Actions and local public builds.

Run:

```bash
docker run --rm \
  -p 80:80 \
  -e CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69' \
  context69:latest
```

This is enough to boot the stack. If runtime settings have not been saved into the app
database yet, the backend starts in degraded mode so you can open the frontend and configure
Qdrant, embedding, Docling, scheduler, and sources there. After saving those settings,
restart the container to activate search and ingest.

In GitHub Actions, these inputs are produced per architecture on native runners and passed to
the Docker image assembly job as artifacts.

## Exposed Services

- frontend via nginx on port `80`
- MCP HTTP on port `8097`

## Frontend-Only Image

Build frontend assets:

```bash
cd frontend
bun run build
```

Build the frontend image:

```bash
cd ..
docker build -f frontend/Dockerfile -t context69-frontend:latest .
```

## Routing

- `/`, `/search`, `/sources`, `/documents/*`: frontend SPA
- `/assets/*`: frontend static assets
- `/v1/*`, `/healthz`, `/openapi.json`: proxied to backend
- `/v1/tasks/stream`: same upstream with SSE hardening in
  `docker/nginx-context69.conf` (`proxy_buffering off`, `proxy_cache off`,
  `X-Accel-Buffering: no`, `proxy_read_timeout/send_timeout 24h`); must stay
  before the generic `/v1/` block so the exact match wins
- `/mcp`: MCP HTTP endpoint

## Multi-replica task streaming (issue 405)

`GET /v1/tasks/stream` is safe behind any number of replicas with no sticky
sessions:

- Every replica runs a resident `LISTEN task_events` hub that fans out into a
  process-local `tokio::broadcast` channel (capacity 1024). PostgreSQL
  `NOTIFY` commits on any replica — Rust writers and pure-SQL paths
  (`cancel`/`trash`/`restore`/`clear`, `maintain_claim_state`,
  `update_external_job` via `tasks`/`task_items` triggers) — are visible to
  every replica. Payload is exactly `task_id`/`item_id`/`status`/`updated_at`.
- Best-effort semantics: each SSE connection first receives a `snapshot`
  frame (full states at subscribe time), then `update` deltas, then `done`
  when every explicitly watched `task_ids` entry is terminal (watch-all
  streams never send `done`). `Last-Event-ID` is not replayed; clients
  re-sync with `GET /v1/tasks` (or `GET /v1/tasks/{task_id}`) on reconnect,
  and lagged broadcast receivers resync the same way. In-band `error`
  frames also mean "resync via `GET /v1/tasks` and reconnect".
- Frontend (`use-task-stream.ts`, cookie `EventSource` with
  `withCredentials`) keeps polling as the fallback: stream errors,
  unavailable `EventSource`, or PAT clients that cannot use cookie SSE fall
  back to the existing 20s queue refresh / settle polling with no behavior
  loss.
- Operator requirements: all replicas must share one PostgreSQL database
  (the `NOTIFY` bus) and one Valkey (sessions/scheduler), and must use the
  same session secret (see `docs/development.md`). The bundled nginx config
  already disables buffering for `/v1/tasks/stream`; when terminating TLS
  or proxying elsewhere, preserve `proxy_buffering off`, `proxy_cache off`,
  `X-Accel-Buffering: no`, and a long `proxy_read_timeout` (24h in the
  bundled config), and forward the session cookie unchanged.
- Smoke test behind nginx: see the `curl -N` checklist in the issue 405
  Task E4 audit note (snapshot first, `update` on task mutation, no
  buffering delay, reconnect resyncs via `GET /v1/tasks`).
