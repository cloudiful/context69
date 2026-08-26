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

### Missing-source cleanup (automatic, with documented data loss)

Legacy direct-path rows that the migration cannot bring onto the
content-addressed layout because the active storage backend has lost their
source object are cleaned up automatically on every normal startup, after the
legacy migration and before task workers resume. This is the documented
data-loss path for source files whose physical RustFS objects are confirmed
absent: there is no manual CLI, and operators do not need to enter the
container or run a one-shot tool.

Selection criteria (deterministic, bounded, restartable):

- `storage_object_id IS NULL` (still pointing at the legacy key),
- `ingest_status IN ('succeeded', 'failed')` (terminal states only),
- `created_at < now() - 24h` (conservative grace so transient S3 races
  during upload, and any late migration writes, are not destructive).

The active storage key is re-checked for each candidate; only an explicit
`NotFound` from the storage abstraction qualifies. Storage errors are
retryable and never result in a delete: they propagate as a per-row
failure, leaving the row in place for the next startup. Qdrant runtime
availability gates the whole run: when Qdrant is not configured, the
cleanup short-circuits and the next startup retries with the same grace
window. This ordering guarantees that the existing application delete
chain (chunk-id Qdrant delete, then document/row deletion, then physical
storage delete) cannot strand PostgreSQL rows without their vector points.

Per-row re-reads between the selection page and the per-row work prevent
racing a concurrent migration or ingest: a row that becomes linked onto
the content-addressed layout (storage_object_id set), whose storage_rel_path
is rewritten, whose ingest_status leaves a terminal state, or whose source
reappears in active storage between selection and per-row work, is skipped
without mutation. Failures are isolated per row; one permanently failing
candidate does not block later rows, and the cursor on (created_at, id)
keeps the loop deterministic across restarts.

The startup log line exposes the run summary:
`scanned`, `confirmed_missing`, `deleted`, `still_present`,
`skipped_recent_nonterminal`, `errors`, and `qdrant_unavailable`.

### Startup old-object cleanup (gated)

The previously-recorded legacy old-key cleanup runs only after the
missing-source cleanup completes successfully and no legacy direct-path
rows remain in the database. The gate is:

- `qdrant_unavailable` and `errors` from the missing-source summary must
  be zero (no per-row failure can leak past the gate),
- `library_files.storage_object_id IS NULL` count must be zero
  (no in-flight legacy row that the missing-source cleanup is still
  about to act on; otherwise old-key deletion would race).

When the gate holds, the existing `cleanup_legacy_objects` is invoked with
its own 7-day grace, backend-matching, live reference, and idempotent
delete/mark guarantees. The startup log line exposes the run summary:
`scanned`, `eligible`, `deleted`, `already_missing`, `skipped_referenced`,
`skipped_backend`, and `errors`. When no candidates ever existed (and no
migration ever recorded a legacy old-key record), this is a no-op.

### Old-object and code cleanup (awaiting explicit confirmation)

Old physical object deletion and removal of the legacy read-compatibility code are
deliberately deferred to a separately reviewed phase that runs only after the
user confirms the startup migration is complete. Do not run a manual cleanup
against production from this workspace, and do not remove the legacy
compatibility code until that confirmation is given.

## Qdrant Cleanup Failure Reproduction (issue 43, phase 0)

Phase 0 of Redmine issue 43 adds a deterministic reproduction fixture for the
observed incident where a Qdrant cleanup failure during library file ingest
currently gets routed through the generic `embedding_vector` dependency gate,
even though the new embedding call never runs. The reproduction is the
regression-safe baseline that the dependency split, error-chain preservation,
batch resume, and UI phases will all be measured against.

The exact call path the reproduction pins:

```
persist_file_sections_for_task (services/library/task_ingest.rs)
  -> persist_sections (services/library/ingest_persistence.rs)
    -> cleanup_ingest_artifacts (services/library/metadata.rs)
      -> QdrantIndex::delete_points_for_library_file
         (src/qdrant_index/cleanup.rs)
      -> QdrantIndex::delete_points
         (src/qdrant_index.rs)
```

When the Qdrant delete errors, `persist_sections` maps the failure to stage
`storage`, `infer_unified_dependency` (services/library/unified_ingest.rs)
matches on the `qdrant` substring, and the existing
`dependency_is_transient` (services/library/dependency_errors.rs) classifies
it as a retryable `embedding_vector` failure. The phase 1+ split changes that
boundary; the reproduction below pins it.

Reproduction layout:

- `tests/qdrant_cleanup_failure.rs` exercises
  `LibraryService::persist_file_sections_for_task` against
  `QdrantIndex::for_test_unreachable` (a deliberately unreachable gRPC
  endpoint) and a spy `EmbeddingProvider`. It is skipped when
  `CONTEXT69_TEST_DATABASE_URL` is not set, matching the other library
  storage integration tests.
- `QdrantIndex::for_test_unreachable` lives in a separate
  `#[cfg(feature = "integration-test-helpers")] impl QdrantIndex` block
  at the bottom of `src/qdrant_index.rs`. The Cargo feature is declared
  in `Cargo.toml` and is not part of any default feature set, so
  production builds (`cargo build`, `cargo build --release`, and the
  deployed binary) never compile the block. Other integration tests
  also stay clean. To exercise the Qdrant-dependent case use
  `cargo test --test qdrant_cleanup_failure --features
  integration-test-helpers` (or
  `cargo test --workspace --features integration-test-helpers`);
  without the feature the file still compiles but that one case is
  skipped with an explanatory message, so `cargo test --test
  qdrant_cleanup_failure` and `cargo test --workspace` remain green
  without extra flags. The constructor still builds the real
  `QdrantIndex` struct so every other invariant is exercised; it only
  skips the `ensure_collection` round trip so the first RPC fails
  deterministically.
- `qdrant_cleanup_failure_aborts_ingest_before_embedding_runs` asserts:
  1. the spy `EmbeddingProvider` was never invoked,
  2. the returned `UnifiedIngestError` is retryable,
  3. the dependency key is currently `embedding_vector` (proving the
     misclassification that phase 1 has to remove),
  4. the underlying `qdrant library file cleanup request failed` context
     is preserved in the error message,
  5. the `library_files` row still exists, because
     `cleanup_ingest_artifacts` deletes SQL rows only after the Qdrant
     delete succeeds.

Classification coverage lives in
`src/services/library/dependency_errors.rs::tests` (timeout / transport /
server-status / permanent validation) and in
`src/services/library/unified_ingest.rs::tests` (`infer_unified_dependency`
on the same error chain). These tests intentionally avoid guessing at the
gRPC error payload format that has not been captured in production.

### Current checkpoint state

The task payload stage checkpoint (`section_payload` on `task_items`)
exists today, but the library file indexer does NOT carry a per-batch
checkpoint for indexing embeddings. The phase 0 reproduction therefore
cannot yet assert "no duplicate vectors are written after a Qdrant cleanup
retry" — phase 3 of issue 43 has to add the per-batch checkpoint before the
cleanup retry can resume without duplicating vectors. The
`reproduction_documents_batch_checkpoint_gap` test in
`tests/qdrant_cleanup_failure.rs` is a grep-able marker for that gap.

### Dependency gate split (issue 43, phase 1)

Phase 1 introduces distinct logical dependency keys `embedding` and `qdrant`
while retaining `embedding_vector` as a compatibility alias. The alias
mapping is centralized in `LibraryDependency::canonical_key` and is used by
gate lookup, `dependency_wait`, probe reservation/recovery, health readiness,
and error routing so existing `embedding_vector` rows/tasks are never
stranded.

- **Canonical keys:** `s3`, `docling`, `embedding`, `qdrant`. Legacy
  `embedding_vector` canonicalizes to `embedding`. New code writes only
  `embedding` and `qdrant`; old tasks with `embedding_vector` are satisfied
  when the canonical `embedding` gate is closed.
- **Error routing:** Qdrant cleanup/upsert/delete failures (messages
  containing `qdrant` plus `connect`/`connection`/`timeout`/`transport`/429/5xx)
  trip the `qdrant` gate. Embedding API failures
  (`embedding upstream transport error`, `runtime is unavailable`,
  `embedding request failed` with 429/5xx) trip `embedding`. Unknown errors
  are never treated as transient.
- **Gate bootstrap:** On startup `refresh_dependency_configuration` ensures
  `embedding`, `qdrant`, and the legacy `embedding_vector` rows exist
  (idempotent `INSERT ... ON CONFLICT DO NOTHING`) and configures all three
  with the same fingerprint so a vector-runtime config change closes both
  gates. `embedding/vector runtime is unavailable` trips both canonical gates
  (and the alias) so the service degrades coherently.
- **Health:** `processing_health` requires `embedding` **and** `qdrant` to be
  `closed` (plus `docling`/`s3` when relevant). For backward compatibility
  the legacy `embedding_vector` gate is synthesized from the canonical
  `embedding` state when the legacy row is absent, so old clients still see
  `embedding_vector: closed` when `embedding` is healthy. The API schema
  (`LibraryDependencyGateResponse`) is unchanged; it simply returns the
  stored gates plus the synthesized alias.
- **Task wait:** `dependency_wait` canonicalizes the requested key before
  lookup, so old items waiting on `embedding_vector` are woken when
  `embedding` recovers. New indexing tasks wait on `qdrant` (and `embedding`
  where both are needed) so Qdrant outages no longer masquerade as embedding
  outages. Probe recovery is independent per gate; success on one does not
  close the other (`no_cross_gate_recovery` is covered in
  `tests/dependency_gate_split.rs`).
- **Qdrant port:** The gRPC endpoint remains `6334` (see
  `qdrant_grpc_url_from_rest_port`); no S3 migration or port change is
  introduced in this phase.

### What phase 0 deliberately does NOT change

- No new `Qdrant` dependency key is added to `LibraryDependency`; the
  embedding/vector gate still absorbs Qdrant errors. The split is phase 1.
- No new Qdrant-specific retry/idempotency ordering; phase 2 owns that.
- No new indexing batch checkpoint; phase 3 owns that.
- No frontend changes; phase 4 owns the UI/i18n updates.

### What phase 1 deliberately does NOT change

- No indexing batch checkpoint or Qdrant retry ordering (phase 2/3).
- No frontend redesign; the health gate list is still returned as
  `Vec<LibraryDependencyGateResponse>` and the synthesized alias keeps the
  previous UI from breaking.

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
