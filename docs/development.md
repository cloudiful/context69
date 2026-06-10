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

Run the backend:

```bash
cargo run
```

Or use the local dev launcher:

```bash
nu scripts/dev.nu backend
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
