# context69

[![Docker Image Workflow](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/publish-docker-ghcr.yml)
[![Release Workflow](https://github.com/cloudiful/context69/actions/workflows/release.yml/badge.svg)](https://github.com/cloudiful/context69/actions/workflows/release.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/cloudiful/context69?sort=semver)](https://github.com/cloudiful/context69/releases)

[中文文档](README.zh-CN.md)

Context69 is a retrieval-focused knowledge base service for ingesting documents, creating vector indexes, and exposing search through HTTP APIs and MCP.

## Features

- Ingest text, URLs, and library files from PostgreSQL-backed sources
- Normalize, chunk, embed, and store vectors in Qdrant
- Support PDF, DOCX, and XLSX conversion through Docling
- Provide multilingual translation and locale-aware retrieval
- Retain task history until users clear it (per-task trash/restore/permanent delete, bulk clear completed/trash); admin cancel-active and Docling recovery stay server-side
- Expose an HTTP API, MCP over HTTP or stdio, and an optional web UI

## Deployment

### Docker image

The published image is available from GHCR. Replace `<tag>` with a release tag such as `v0.8.1`.

Requirements:

- PostgreSQL for application data
- Qdrant and an embedding provider for search and ingestion
- Valkey is optional for shared sessions and scheduler coordination
- Docling is optional and required for PDF, DOCX, or XLSX conversion

```bash
docker volume create context69-library

# Generate the master secret once, keep it somewhere safe, and pass the same
# value to every replica.
export CONTEXT69_MASTER_SECRET="$(openssl rand -base64 32)"

docker run -d \
  --name context69 \
  --restart unless-stopped \
  -p 80:80 \
  -p 8097:8097 \
  -v context69-library:/app/data/library \
  -e DATABASE_URL='postgres://user:password@db/context69' \
  -e CONTEXT69_APP__MASTER_SECRET="$CONTEXT69_MASTER_SECRET" \
  ghcr.io/cloudiful/context69:<tag>
```

Open `http://localhost` after the first start. Configure Qdrant, the embedding provider, and any optional services in the Settings page, then restart the container. Port `80` serves the web UI and HTTP API; port `8097` serves MCP over HTTP.

The Runtime settings section has a Qdrant Test Connection button for the unsaved URL. It checks reachability over the same gRPC transport indexing uses (usually port `6334`, not the REST `6333` endpoint) with a 5s timeout, answers success or a redacted failure, and never saves settings, notifies observers, or requires the collection to exist.

### Stored secrets

`CONTEXT69_APP__MASTER_SECRET` is a Base64-encoded 32-byte key that encrypts every stored secret — provider API keys, source connection URLs, Git credentials, webhook secrets, and the browser session key. Generate it once, store it with the same care as any other credential, and keep it: without it, encrypted secrets cannot be read back. Rotating it requires the configured key-rotation run, not a restart.

Setting it on a deployment that already has secrets converts what it finds. At startup, before anything reads a credential, Context69 encrypts every secret still stored in the legacy plaintext form, in batches, using the configured key. The work is per-row and per-batch atomic: if it stops part way, the batches that already finished stay encrypted and the next startup continues from what is left. A startup fails if a stored secret cannot be encrypted — for example one whose key this build does not recognise — so a deployment never runs believing its secrets are protected when they are not. A deployment with no master key configured is unaffected: its secrets stay readable in plaintext exactly as before.

Two things this does not do. It is a startup conversion, not continuous protection, so **every replica that writes secrets must be given the same master secret** (or be stopped while one replica converts); an instance running without the key can write a new plaintext secret after the conversion has finished. And backups taken before the conversion still contain plaintext secrets — **context69 does not scrub existing backups**, so treat them accordingly and retire them on your own schedule.

### Build from source

```bash
cargo build --release --bin context69
cd frontend
bun install --no-save
bun run build
cd ..
mkdir -p ci-image-input/frontend-dist
install -Dm755 target/release/context69 ci-image-input/context69
cp -R frontend/dist/. ci-image-input/frontend-dist/
docker build -t context69:latest .
```

Run the locally built image with the Docker command above and replace the image name with `context69:latest`.

## Backend tests

Pure tests run anywhere:

```bash
cargo test --workspace
```

Database-backed tests are explicitly marked `#[ignore]`, so the command above
runs only the pure ones and reports the ignored ones instead of pretending they
passed. The configured-database runner and its explicit reset flow are shown below.

A normal database run first executes
`cargo test --workspace --no-fail-fast --offline` with Cargo's default
parallelism, including doctests. Ignored database and opt-in tests stay
unselected. A pure-suite failure stops the run before the database target is
resolved or any database preflight, lock, or reset begins. `--preflight` and
`--list` remain inspection modes and do not run the pure suite.

The runner reads its target from `CONTEXT69_TEST_DATABASE_URL`, or from
`DATABASE_URL` in the repository `.env` file when that variable is unset. It
never takes a database URL as an argument, never creates or drops a database,
and never provisions a PostgreSQL server. An ambient `DATABASE_URL` on its own
does not select a target.

`./scripts/test-backend.sh --preflight` inspects the target's schema, migration
ledger, and extensions without writing anything, and reports whether the runner
may reset it. `./scripts/test-backend.sh --list` reconciles the checked-in
database-test inventory against the compiled tests without connecting at all.

Resetting the target is a separate, explicit consent, and the wrapper forwards
it unchanged rather than adding it for you. The runner only drops and recreates
the `context69` schema when you ask for it:

```bash
./scripts/test-backend.sh --preflight         # read-only, report the scope
./scripts/test-backend.sh --authorize-reset    # pure suite, reset, migrate, then DB tests
```

By default, any other client connection blocks reset. In an operator-confirmed,
single-user test window, use `--allow-idle-connections` on both preflight and run
to permit only visible idle sessions with no open transaction. Active, transactional,
masked/unknown sessions and prepared transactions still block reset. Idle sessions
may become active after inspection: never use this option with application writers.
The option is invalid with `--list` and does not grant reset consent.

```bash
./scripts/test-backend.sh --preflight --allow-idle-connections
./scripts/test-backend.sh --authorize-reset --allow-idle-connections
```

Run artifacts and per-target logs are written to
`target/backend-test-results/<run-id>/`.

## License

Apache-2.0. See [LICENSE](LICENSE).
