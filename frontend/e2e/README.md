# End-to-end smoke tests

Standalone [Playwright Test](https://playwright.dev/) harness for full-browser
smoke coverage of the frontend. Vitest stays the component/composable runner;
Rust stays the API/integration runner.

## Prerequisites

- A disposable local stack: `./scripts/dev.sh full` (frontend `:5173`, API
  `:8096`). The harness targets a running stack; it does not start one.
- Chromium: `bunx playwright install chromium` (plus `--with-deps` on a bare
  Linux host).
- A disposable local database whose bootstrap account still uses the documented
  `admin` / `change-me-now` credentials.

## Run

```sh
cd frontend
bun run test:e2e
```

Before any spec runs, `e2e/global-setup.ts` polls the frontend and the API
`/healthz` endpoint. It fails with one actionable message when the stack is not
reachable. A degraded API is accepted as long as the database is up, so Qdrant,
Docling, and embedding services are not prerequisites.

`e2e/auth.setup.ts` signs in once through the real login UI and writes the
session cookie to `e2e/.auth/user.json`, which the `chromium` project reuses.
That file is generated at test time and never committed.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `E2E_BASE_URL` | `http://127.0.0.1:5173` | Frontend origin under test |
| `E2E_API_URL` | `http://127.0.0.1:8096` | API origin used by the readiness probe |
| `E2E_USERNAME` | `admin` | Login name for the auth setup |
| `E2E_PASSWORD` | `change-me-now` | Password for the auth setup |
| `E2E_STORAGE_STATE` | `e2e/.auth/user.json` | Generated storage-state path |
| `E2E_READINESS_TIMEOUT_MS` | `60000` | Readiness polling budget |

Never point these at a shared or production environment.

## Intentionally out of scope

- No Qdrant, Docling, embedding, or external-service coverage.
- No visual snapshot baselines or cross-browser matrix; Chromium only.
- No exhaustive page-by-page duplication of the Vitest suite.
