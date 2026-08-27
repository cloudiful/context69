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

### Error observability and cleanup retry (issue 43, phase 2)

Phase 2 preserves accurate, bounded Qdrant diagnostics and makes the ingest
cleanup retry semantics explicit without adding batch checkpoints or broad
frontend changes.

- **Operation-specific Qdrant context:** Every Qdrant boundary (`upsert_points`,
  `delete_points`, `delete_points_for_library_file`, `update_points_batch`,
  `search_points`, `count_points`) wraps failures with `operation`, `collection`,
  `category` (`timeout` / `transport` / `server` / `rate_limited` /
  `client_error` / `provider_unknown`), and a bounded `underlying_preview`
  (800 chars). The outer message keeps the legacy `qdrant ... request failed`
  prefix so existing `qdrant` substring + transport/status classification stays
  compatible. Timeout is distinguishable (`category=timeout`, `timed out after
  30s`). Provider status is only claimed when present in the error chain
  (`429`, `5xx`, `transport`, etc.); otherwise `provider_unknown`.
- **Safe and bounded:** No request payloads, API keys, or document text are
  included. Only counts (`batch_size`, `point_count`, `payload_count`) and
  identifiers (`collection`, `file_id`) are emitted. Underlying chain is
  truncated via `truncate_for_qdrant_error` (800 chars) and also chain-preserved
  for `dependency_is_transient` classification. `redact_dependency_error`
  still caps at 1000 chars for gate persistence.
- **Idempotent deletes:** `delete_points` early-returns on empty slices.
  Qdrant's own semantics already treat missing point ids / zero-match filters
  as success. An explicit helper `is_qdrant_idempotent_not_found` documents
  the narrow swallow boundary (point/filter `not found` without
  `permission`/`validation`/`authentication`) and is tested not to swallow
  `PermissionDenied`/`validation` errors. Collection-not-found without point
  hint is not swallowed.
- **Cleanup ordering:** `cleanup_ingest_artifacts` (services/library/metadata.rs)
  does Qdrant `delete_points` (chunk ids) then Qdrant
  `delete_points_for_library_file` (payload filter orphan) **before**
  `delete_documents_for_library_file`. On Qdrant failure it returns
  `anyhow::Error` that `persist_sections` maps to `LibraryIngestFailureStage::Storage`
  and `infer_unified_dependency` routes to `qdrant`; with transport/timeout/5xx
  it is retryable under the `qdrant` gate. SQL `document_chunks`/`documents`
  remain intact for retry – the `library_files` row survives and no new
  `embedding.embed_texts` call occurs (pinned by
  `qdrant_cleanup_failure_aborts_ingest_before_embedding_runs` and the new
  `qdrant_cleanup_failure_preserves_sql_and_is_retryable_with_operation_context`
  in `tests/library_ingest_retry.rs`).
- **Embedding vs Qdrant:** `embedding` errors (`embedding upstream transport error`,
  `embedding request failed` with 429/5xx) never contain `qdrant`; `qdrant`
  errors always contain `qdrant` and transport/status signals. The new formatter
  never mixes the two substrings. Tests in `qdrant_index::tests` and
  `library_ingest_retry.rs` assert operation labels, bounded previews, category
  distinction, and non-cross-routing.
- **Batch checkpoint gap:** Still not addressed – phase 3 owns it. The existing
  `reproduction_documents_batch_checkpoint_gap` marker remains.

### Indexing batch checkpoint (issue 43, phase 3)

Phase 3 adds a durable, bounded, lease-conditional batch checkpoint under the
existing task item payload (`indexing_checkpoint` key). Evaluation of
payload vs dedicated table concluded that the existing `task_items.payload`
plus `set_task_item_payload` (lease_token + status=running) already provides
lease-conditional, bounded, and concurrent-safe semantics, keeps the change
small, and avoids a new migration. The dedicated table would duplicate the same
lease handling and is deferred.

- **Stored fields only:** `v` (version=1), `next_batch_index` (usize),
  `total_batches` (Option<usize> for observability), `record_hash` (Option<String>
  hex sha256 of prepared record_hash chain). No full text, embeddings, or
  document payload is ever stored in the checkpoint. The JSON is capped at
  512 bytes and preserves all existing payload keys (`section_payload`,
  `file_id`, etc.) via a cloned payload insert.
- **Old payload compatibility:** `parse_indexing_checkpoint` returns
  `next_batch_index=0` when the key is absent or version mismatches, so old
  tasks start at batch 0. A hash mismatch (section_payload changed) resets the
  checkpoint to 0 and forces a fresh `cleanup_ingest_artifacts` before any new
  batch.
- **Cleanup ordering:** `cleanup_ingest_artifacts` (Qdrant `delete_points` +
  `delete_points_for_library_file` before `delete_documents_for_library_file`)
  runs once per file when `next_batch_index==0`. On resume (`>0`) it is
  skipped so already checkpointed batches are not deleted. The per-document
  `delete_document_chunks` in the new checkpoint path is also skipped on resume;
  SQL inserts are idempotent (duplicate key treated as success) and Qdrant
  `upsert_document_chunks` is idempotent via deterministic `chunk_id`
  (`chunk_uuid` = `source_key:external_id:record_hash:chunk_index`).
- **Per-batch contract (at-least-once, not exactly-once):** For each batch
  the order is `embed_texts` → `insert_document_chunks` (SQL) →
  `upsert_document_chunks` (Qdrant) → `set_task_item_payload` advancing
  `next_batch_index`. The checkpoint is only bumped after both stores succeed.
  On **SQL success / Qdrant failure** the checkpoint stays behind; retry
  re-inserts the same chunk_ids idempotently. On **Qdrant success / checkpoint
  failure** (lease lost) the checkpoint also stays behind; retry re-upserts the
  same points idempotently. This is explicitly **not** full transactional
  exactly-once across external Qdrant; it is at-least-once external upsert with
  deterministic idempotency, and is documented and re-tested as such.
- **Lease-conditional and monotonic:** `payload_with_checkpoint` rejects
  regressions (`next <= old`), overshoot (`next > total`), oversized JSON, and
  uses `set_task_item_payload(item_id, lease_token, payload)` which checks
  `lease_token` and `status=running`. Concurrent workers cannot regress or
  overwrite; a lost lease yields `task item lease was lost while checkpointing
  batch N` (retryable). `handle_task_ingest_failure_with_payload` skips the
  post-failure `cleanup_ingest_artifacts` when `next_batch_index>0` so committed
  batches are not deleted.
- **Finalize:** `replace_file_documents` and `bump_search_generation` plus
  `update_file_status(Succeeded)` and dependency gate closes happen only after
  all batches and mappings are persisted. The `library_files` row and
  translation/extraction enqueue flow is preserved.
- **Tests:** `tests/indexing_batch_checkpoint.rs` (unit + integration behind
  `integration-test-helpers` + `CONTEXT69_TEST_DATABASE_URL`) covers resume after
  batch 1 (no re-embedding), checkpoint loss (lease-conditional failure leaves
  Qdrant idempotent), deterministic duplicate IDs, old payload compatibility,
  and cleanup-once ordering. `tests/library_ingest_retry.rs` continues to pin
  Qdrant-before-SQL ordering and gate split.
- **Module layout:** The checkpoint types and pure helpers live in
  `src/services/library/ingest_checkpoint.rs` (parse/serialize, batch
  estimation, prepared-record hashing, payload cloning with monotonicity
  guard, bounded size). The driver that ties them to
  `set_task_item_payload` (`persist_file_sections_for_task_with_checkpoint` and
  its inner batch/finalize helpers) lives in
  `src/services/library/ingest_checkpoint_persistence.rs`. The remaining
  per-task item surface (`prepare_file_sections_for_task`,
  `persist_file_sections_for_task`, failure handling, etc.) stays in
  `src/services/library/task_ingest.rs`.
- **Limitations:** Not a distributed transaction; operators should not expect
  exactly-once. If the process dies between Qdrant and checkpoint, the same
  batch will be upserted again on retry – safe because IDs are deterministic
  and Qdrant upsert is idempotent, but at-least-once is the accurate contract.

### High-confidence index fixes (issue 50, phase 2)

Phase 2 lands two high-confidence fixes observed in the phase 1 root-cause
assessment (Redmine note 889):

- **SQL no-op for unchanged chunk hash:** `update_library_business_fields.sql`
  now guards the `document_chunks` update with a null-safe
  `record_hash IS DISTINCT FROM $7` predicate. The parent `documents` rewrite
  stays unconditional so extracted metadata publishing may still change
  `external_id` / `source_uri` / `published_at` / `updated_at_source` /
  `metadata_json` independently of `record_hash`. Observed removed
  write-amplification: document 49532 with 175 chunks dropped from 175–296 row
  updates (3.9–5.3s) to zero chunk rewrites whenever the hash is unchanged.
  Regression coverage lives in `tests/document_business_fields.rs`.
- **Qdrant `library_file_id` keyword payload index:** the typed
  `PAYLOAD_FIELD_INDEXES` table in `src/qdrant_index/collection.rs` adds
  `library_file_id` (Keyword) to the baseline schema-preserving field/index
  list (Keyword for `group_key` / `group_path` / `visibility` / `source_key`,
  Integer for `group_id` / `document_id` / `published_ts`). New collections
  build every entry through `create_field_index`; `QdrantIndex::connect`
  additionally calls `ensure_library_file_id_field_index` for pre-existing
  collections so the cleanup filter is fast on startup (avoids the 30s
  Qdrant timeout observed on the 330k-chunk collection).
  `ensure_field_index_idempotent` only swallows gRPC `AlreadyExists` paired
  with field-index-shaped text; permission / validation / transport failures
  still propagate through the existing operation/collection/category error
  formatter. Unit coverage sits next to `metadata_payload_key` in
  `src/qdrant_index/collection.rs`.

No production DB / Qdrant / S3 mutation. Production readiness for the SQL
change requires `cargo sqlx prepare --workspace` against a fully-migrated
scratch database (the hash for the new SQL replaces the offline cache entry).
No new persistent migration is shipped: the Qdrant payload index is external
to PostgreSQL.

### Two-tier task dispatch (issue 50, phase 3)

Phase 3 splits the dispatcher hot path into a fast claim statement and a
periodic maintenance statement so notification-driven wakes avoid the
exhausted/expired UPDATE/RETURNING work that previously ran on every claim.
The existing lease/retry invariants are preserved verbatim, the public
`Database::claim_items` API still observes the same behavior, and no new
precheck is introduced.

- **`claim_items.sql` (fast path):** eligible selection with `FOR UPDATE
  OF ti SKIP LOCKED`, parent activation for claimed `task_id`s only,
  item lease/attempt fields, `task_attempts` insert, and the returned
  `ClaimedItem`. The expired-attempt interruption is scoped to the items
  being claimed so a notification wake that picks up an item with an
  abandoned lease still recycles that worker's attempt without waiting
  for the recovery tick. Maintenance UPDATE/RETURNING CTEs do not run.
- **`maintain_claim_state.sql` (recovery path):** exhausted
  item/file/task propagation and a global expired-attempt interruption that
  atomically revokes the `task_items` lease (`lease_token`/`lease_until`)
  for expired running items with an active parent task (`queued`/`running`
  or due `waiting`) before interrupting the matching `task_attempts` row.
  The item stays `running` so the fast claim can reclaim it, but the old
  token can no longer `finish`/`heartbeat`/`progress`. The expired item
  predicate is `lease_until IS NULL OR lease_until < now()` and active
  parent task filtering applies (the item is only revoked when its parent
  task is `queued`, `running`, or due `waiting`). Idempotent and safe to
  repeat: only rows that already satisfy the exhausted
  (`attempt_count >= 5` while item is `queued`/`waiting`) or expired
  (`lease_until IS NULL OR lease_until < now()` while item is `running`
  and the parent task is active) predicates are touched, and
  `expired_attempts` counts only the interrupted attempt rows. Returns a
  small row of counts so the dispatcher can log recovery work without an
  extra round trip.
- **`Database::claim_items` (compatibility):** runs `maintain_claim_state`
  and the fast claim inside a single PostgreSQL transaction so existing
  callers and the lease/retry tests in `tests/task_lease_invariant.rs`
  observe the same exhaustive behavior the old monolithic statement
  provided. Tests that call `claim_items` directly therefore still see
  exhausted items become terminal and expired attempts recycled even
  when the dispatcher is not running.
- **`Database::claim_items_fast`:** fast claim only. Dispatcher calls
  this on every notification-driven wake and on worker-spawn wakes; the
  hot path is bounded by the available worker semaphore and is safe to
  spam. `Database::maintain_claim_state` is the only place the exhausted
  /expired UPDATE/RETURNING work runs on the dispatcher side.
- **Dispatcher wiring (`src/services/tasks/dispatcher.rs`):**
  - Startup runs `maintain_claim_state` once and then a fast dispatch.
  - Two `select!` arms split the work: the `notified` arm drains the
    queue with `claim_items_fast` only, and the `recovery.tick()` arm
    runs `maintain_claim_state` sequentially before a fast drain so the
    claim sees freshly converged state. `MissedTickBehavior::Delay` is
    set and the immediate interval tick is consumed at startup so the
    first periodic recovery fires 30 seconds later, not milliseconds
    after startup. Notification wakes never run maintenance; only the
    recovery tick does, and failures are logged without stalling the
    loop.
  - Maintenance is sequential with the recovery dispatch inside one
    recovery cycle and idempotent if retried. Exhausted propagation
    touches rows the fast eligible selection explicitly excludes
    (`attempt_count >= 5`), and expired-attempt interruption only
    touches abandoned leases, so concurrent callers remain safe to retry
    even if a notification-driven fast claim races an out-of-band
    `maintain_claim_state` call. Both arms eventually converge because
    maintenance runs on every recovery tick regardless of how many
    notifications fire in between.
  - Worker semaphore behavior, notify coalescing, and the existing
    `Notify`/dispatch state are preserved; only the call site changed
    from `claim_items` to `claim_items_fast`/`maintain_claim_state`.

Exhausted-only queues (every item has already hit the attempt cap) still
converge because the recovery tick runs `maintain_claim_state` before the
fast dispatch even when the fast dispatch sees no claimable rows. A
`has_claimable_items` precheck would skip that maintenance exactly when
it is most needed, so it is deliberately not added: maintenance is the
recovery path, not the dispatch path.

Regression coverage lives in `tests/task_dispatcher_fast_path.rs`:
fast claim claims and activates correctly, maintenance-only converges
exhausted item/task/file when no claimable row exists, maintenance-only
interrupts expired attempts while revoking the item lease and rejecting
late `finish`/`heartbeat`/`progress` with the stale token/attempt and
leaving the item reclaimable, the compatibility path still converges
exhausted state and recycles expired attempts, and the maintenance
outcome struct returns zeros on an idle database. Existing
`tests/task_lease_invariant.rs` cases are unchanged because they call
`claim_items` and therefore exercise the compatibility path.

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
