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

## Local Full-Stack Flow

1. Start the backend with `cargo run`
2. Start the frontend with `bun run dev`
3. Regenerate API types when the OpenAPI contract changes
