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

Forgejo CI uses `Dockerfile.forgejo` instead. That file uses
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
- `/mcp`: MCP HTTP endpoint
