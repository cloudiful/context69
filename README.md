# context69

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Latest Release](https://img.shields.io/github/v/release/cloudiful/context69?sort=semver)](https://github.com/cloudiful/context69/releases)

[中文文档](README.zh-CN.md)

Context69 is a retrieval-focused knowledge base service for ingesting documents, creating vector indexes, and exposing search through HTTP APIs and MCP.

## Features

- Ingest text, URLs, and library files from PostgreSQL-backed sources
- Normalize, chunk, embed, and store vectors in Qdrant
- Support PDF, DOCX, and XLSX conversion through Docling
- Provide multilingual translation and locale-aware retrieval
- Retain task history until users clear it (per-task trash/restore/permanent delete); the processing queue offers cross-page task selection with contextual bulk retry/resume/cancel/trash, and confirmed bulk restore or permanent delete in the trash view; admin cancel-active and Docling recovery stay server-side
- Delete library files, folders, sources, and documents logically: their search tombstones commit before any vector purge, so they disappear from search immediately, and a failed or interrupted purge is retried without blocking the delete. Each tombstone records the Qdrant endpoint and collection that its delete committed against, and a retry removes points only from that recorded origin, so activating a different collection cannot redirect it. Tombstones written before an origin was recorded keep no origin and stay pending for manual verification instead of being guessed from the current settings.
- Expose an HTTP API, MCP over HTTP or stdio, and an optional web UI
- Authorize through one shared policy engine built at startup: administrator gates use the caller's current role, group decisions combine the live SQL role with the exact personal-access-token group range, and the six token scopes are exact credential grants. Public group discovery stays readable without membership, and a new group needs no engine rebuild.

### Runtime chunking

The chunking budget is measured in characters or in tokens. Token mode uses an
explicit tokenizer (`cl100k_base`, `o200k_base`, a pinned preset, or a verified
local Hugging Face `tokenizer.json` artifact), and the overlap stays inside the
budget of a single chunk. One tokenizer control in the chunking section measures
the token budget, so the chunk budget and its token counting never disagree. A
saved budget governs
new tasks immediately, without a restart; a task that already started keeps the
budget it was created with, and stored documents are never rechunked
automatically.

A pinned preset is a fixed upstream `tokenizer.json`: its repository, revision,
size, SHA256, and license are constants. The runtime images bundle the
EmbeddingGemma 2 tokenizer (Apache-2.0) at
`<artifact-dir>/<lowercase-sha256>.json`, so it is verified and ready offline at
startup without any preparation, download, or network access. Attribution, the
pinned revision, and the exact hash are listed in `THIRD_PARTY_NOTICES.md`,
which ships in the image beside the Apache-2.0 license text.

A bare installation without that bundled artifact keeps the prepare path:
preparing the selected preset retrieves only that file into the writable cache
directory, verifies the exact size and hash before anything becomes usable, and
activates it in the running process without a restart. The license and source
links are shown before the preparation starts. The cache directory is
`./data/tokenizers.cache`, and `CONTEXT69_CHUNKING__TOKENIZER_DOWNLOAD_DIR`
overrides it; a prepared preset loads from that verified cache on restart
without network access. Readiness is per process, so every replica needs the
bundled artifact or the verified cache file to use the preset.

Mounted artifacts remain the advanced path for operator-provisioned files:
place each standard `tokenizer.json` at
`<artifact-dir>/<lowercase-sha256>.json` and mount the directory read-only; the
default is `./data/tokenizers` and
`CONTEXT69_CHUNKING__TOKENIZER_ARTIFACT_DIR` overrides it. The process verifies
the name hash and the readable tokenizer at startup, offers only verified
artifacts in Settings, and fails closed when a selected hash is missing. Adding
or removing a mounted artifact requires a restart, and a hash that an existing
task pinned must stay available while that task can retry.

### Embedding settings

Document and code embedding are separate settings domains. The document model
holds its endpoint, credential, model, optional query model, dimensions, and
timeout. The optional code embedding block stores sparse overrides: a blank
endpoint, model, query model, dimension, or timeout inherits the document
value field by field, and a stored code credential is used only while the
endpoint stays on the same origin, where the document credential applies
instead. A blank query model inherits the corresponding document or code model.

Changing an endpoint, model, or dimensions changes that vector space. The
existing collection keeps serving until an explicit rebuild activates its
target; while the configured and active identities disagree, search and
ingestion stay unavailable, so the operator can still cancel the rebuild and
keep the current collection. A fresh installation has no collection to keep, so
its first index build is always explicit. After a verified activation, the
replaced physical collection is scheduled for automatic deletion once a
five-minute drain grace period has passed. The background maintenance retries a
cleanup failure with capped backoff across restarts and treats an
already-absent collection as success. It deletes only from the endpoint recorded
with that retired collection, and it never deletes a collection whose owning
endpoint cannot be proven, including a legacy identity without a recorded
endpoint or a matching fingerprint. Historical collections that predate this
behavior are never backfilled or swept.

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

Before upgrading, stop every old writer and back up PostgreSQL and the Qdrant collections. Run the new database initialization and migrations, then start the new version; do not mix old and new writers. The task execution protocol upgrade preserves task and attempt history, but older binaries refuse the upgraded schema and cannot be rolled back against it.

During graceful shutdown, a worker waiting on a persisted Docling conversion can yield its own claim after reaching a safe checkpoint. The next worker adopts the existing remote job without consuming a retry. Other stages continue normally within the shutdown budget; if shutdown times out, the database is unavailable, or the process is killed, unfinished leases recover after expiry rather than immediately.

The Runtime settings section has a Qdrant Test Connection button for the unsaved URL. It checks reachability over the same gRPC transport indexing uses (usually port `6334`, not the REST `6333` endpoint) with a 5s timeout, answers success or a redacted failure, and never saves settings, notifies observers, or requires the collection to exist.

The Embedding section calls the existing model the document model and accepts an optional query model that inherits the document model when left blank. Document and code embedding keep their own vector collections; a code or image block that stores no override of its own runs in the document vector space.

The optional Code embedding block configures a Git-oriented model for code chunks and code search. It is disabled by default and stores sparse overrides: a blank endpoint, model, query model, dimension, or timeout inherits the document value, and a stored code credential applies only while the endpoint stays on the same origin, where the inherited document credential is used instead. Disabling it retains the saved non-secret fields and the code credential unless the operator explicitly removes that credential; enabling or changing it requires an explicit Git repository reindex.

The optional Image Embedding block configures a model for stored image bytes and image queries. It is disabled by default and stores the same sparse overrides: a blank endpoint, model, query model, dimension, or timeout inherits the document value, while the collection namespace falls back to `context69_images`. Disabling it retains the saved fields and the image credential unless the operator explicitly removes that credential. Its Test Connection button probes the unsaved image route with the same sparse values a save would store and never saves, and the physical collection is derived per runtime from the configured namespace.

Image ingestion has its own optional embedding settings and writes its own fingerprint-scoped Qdrant collection; it never changes document or code settings and never adopts their indexes. PNG, JPEG, and WebP only are accepted after magic-byte, media-type, dimension, and size checks, and originals are retained: image submissions reject `release_after_processing`, translation, and text-conversion options. A URL import keeps the existing SSRF, peer, redirect, and size protections. Submit standalone images, or link an occurrence to an existing parent document by key:

```rust
use context69_sdk::{ImageBatchItem, ImageBatchRequest, ImageSearchRequest};

client
    .submit_image_batch(
        "team/library",
        &ImageBatchRequest {
            items: vec![ImageBatchItem {
                filename: "photo.png".into(),
                media_type: "image/png".into(),
                content_base64: Some(encoded_png),
                url: None,
                declared_sha256: None,
                folder_id: None,
                parent: None,
                role: None,
                ordinal: 0,
                caption: None,
                context: None,
                source_uri: None,
                options: Default::default(),
            }],
        },
    )
    .await?;

let hits = client
    .search_images(
        "team/library",
        &ImageSearchRequest {
            query: "a red bicycle in a city".into(),
            limit: 20,
            media_type: None,
        },
    )
    .await?;
```

Set `parent` to the target document key and `role`/`ordinal` to attach a cover or body occurrence; omission keeps the image standalone. Visual search embeds the text query into the image model's vector space and returns authorized images with score, file identity, and current parent occurrences. It does not perform OCR or HTML extraction, so text printed inside an image is not searchable through this channel. Hydration drops stale or deleted candidates and may return fewer hits than `limit`; the response has no total or cursor. Use `list_document_images` and `unlink_document_images` to inspect up to 512 occurrences (`has_more` reports the bound) and remove links only; files and vectors stay intact.

Each Git repository selects its own index profile. `Lexical` is the default and searches the stored code text only. `Hybrid` and `FullSemantic` require the code model and write an isolated code vector set for each indexed generation: Hybrid fuses lexical and vector ranks, while FullSemantic ranks by code-vector similarity. When the code model is not configured, the serving generation was indexed without code vectors, the recorded code destination no longer matches the current code identity, or the vector runtime is unavailable, a semantic request returns lexical results and reports the reason through `vector_unavailable` instead of claiming a semantic result. Code vectors use their own collection and never reuse document vectors or the document collection; the code credential is its own unless the code block stores no credential of its own, in which case the document credential applies on the same origin. Changing the code model requires an explicit reindex; existing repositories keep their configured profile until it is changed. The tracked ref, optional pinned commit, and index profile remain editable after registration: changing any of them marks the stored snapshot stale, and an index attempt started against the previous target or profile cannot publish. A source whose last index attempt failed reports a bounded `last_error_code` classification, never a provider message, response body, or secret-bearing detail.

Refresh is manual by default. An optional per-repository interval runs independently of the document pipeline, and authenticated provider pushes refresh the repository through `/v1/webhooks/{provider}/{external_hook_id}`; the webhook requires the registration's signing secret, verifies the raw body with HMAC-SHA256, and only a matching push event for the source's repository and tracked ref dispatches work. A request that arrives while indexing is already running leaves a durable follow-up instead of being dropped, and one repository never runs two index tasks at once. Webhook deliveries are idempotent by provider delivery id, and unsigned, mis-signed, foreign-repository, and unrelated-ref deliveries are acknowledged without refreshing.

Successful generations are retained up to the configured total, which includes the active snapshot: the default of 2 keeps the active snapshot and one previous one, and the minimum of 1 keeps only the active one. Older, failed, and abandoned generations are pruned after their code vectors are deleted. Pending vector cleanup is retried by independent maintenance even for manual-only or disabled sources, without waiting for another index; the generation keeps a pending-cleanup record instead of losing the only reference to its remote points. Deleting a repository disables it immediately, fences new index work, refuses deletion while a task is still active, removes its vectors and metadata once no writer remains, and retries later if cleanup cannot complete.

The group **Git** tab is the user-facing surface for this lifecycle. Group maintainers register provider connections and repositories, then edit the tracked ref, pin, index profile, include/exclude globs, interval, webhook switch, retention, and enabled state. Sync keeps the same-target no-op, while Rebuild sends an explicit force request that reconstructs the current snapshot even when the commit and pipeline identity are unchanged. Deleting a repository is confirmed and refreshes the list. Group viewers can browse indexed files, read bounded content windows, and run code search with generation, commit, and ref provenance, but cannot change sources or connections.

Changing the embedding model, gateway, or dimensions changes the vector space. The existing collection keeps serving until an explicit Rebuild Vector Index completes and activates its target; while the configured and active identities disagree, search and ingestion stay unavailable, so the operator can still cancel the rebuild and keep the current collection. A completed verified activation schedules only the collection it actually replaced for automatic deletion after a five-minute drain grace period; cleanup retries independently with capped backoff and never marks a successful rebuild failed. A fresh installation has no collection to keep, so its first index build is always explicit. Rebuild status, pause, resume, and cancel are durable and admin-only: a pause lets the committed batch finish before the rebuild parks, a resume continues after the last written page, and a cancel stops the attempt without touching the existing collections. Rebuilds stream one document at a time, staging and embedding each document in bounded batches before the next one advances; progress reports the documents already staged and the chunks already embedded, and a chunk total appears only once the staged total is known. After the staged vectors are verified, the rebuild installs the staged original chunks into the live table in bounded batches after a one-time reset, so activation itself performs no publication-sized mutation. Search and ingestion stay unavailable from that reset until a successful publish clears the durable invalidity; a cancelled or failed installation keeps the index invalid until a fresh successful rebuild. Completed and cancelled staging is drained in bounded batches by the existing maintenance tick. Token budgets created by this version use the `tokens_v2` cutting path, which older binaries cannot execute: finish or cancel any `tokens_v2` task before rolling back, because an older binary cannot resume it and does not silently remap it. The processing queue can separately pause new work; running tasks continue.

Registry capacity counts retained point ids rather than vector storage. A replacement discounts only the original ids that its producer recorded before deleting them, and only once the entry belongs to an inactive collection. Legacy entries without that recorded provenance remain fully reserved, so a tight capacity limit can require operator review or a fresh rebuild instead of a guessed discount.

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

## Tests

The backend and frontend suites contain only fast, isolated unit tests. They run without PostgreSQL, Qdrant, network services, a browser, or external processes.

```bash
cargo test --workspace
cd frontend && bun run test
```

Frontend type checking and the production build:

```bash
cd frontend && bun run check
cd frontend && bun run build:with-api
```

End-to-end behavior is verified by manual acceptance against a running deployment, not by an automated integration, database, or browser suite.

## License

Apache-2.0. See [LICENSE](LICENSE).
