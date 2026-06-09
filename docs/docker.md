# Docker Deployment

## All-in-One Image

The root `Dockerfile` builds:

- the frontend assets
- the Rust backend
- an nginx + application runtime image

Build:

```bash
docker build -t context69:latest .
```

Run:

```bash
docker run --rm \
  -p 80:80 \
  -e CONTEXT69_APP_DB__URL='postgres://user:pass@db/context69' \
  -e CONTEXT69_EMBEDDING__API_KEY='sk-xxx' \
  context69:latest
```

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
