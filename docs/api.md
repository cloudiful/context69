# API Reference

## Main Endpoints

- `GET /healthz`
- `GET /openapi.json`
- `GET /v1/auth/me`
- `GET|POST /v1/auth/personal-access-tokens`
- `DELETE /v1/auth/personal-access-tokens/{token_id}`
- `POST /v1/search`
- `GET /v1/documents/{document_id}`
- `POST /v1/scopes/ensure`
- `POST /v1/groups/by-path/{group_path}/batch/text`
- `POST /v1/groups/by-path/{group_path}/batch/url`
- `POST /v1/groups/by-path/{group_path}/batch/file`
- `POST /v1/groups/by-path/{group_path}/library/files/upload` (multipart)
- `POST /v1/groups/by-path/{group_path}/library/files/prepare-upload`
- `POST /v1/groups/by-path/{group_path}/library/files/{file_id}/release-source`
- `POST|GET /v1/tasks`
- `GET /v1/tasks/{task_id}`
- `GET /v1/tasks/{task_id}/items`
- `POST /v1/tasks/{task_id}/retry`
- `POST /v1/tasks/{task_id}/rerun` (admin-owned terminal task; creates a fresh task from the unfinished items, bypassing the idempotency binding)
- `POST /v1/tasks/{task_id}/cancel`
- `POST /v1/tasks/{task_id}/trash`
- `POST /v1/tasks/{task_id}/restore`
- `DELETE /v1/tasks/{task_id}` (permanently deletes a trashed task)
- `POST /v1/tasks/clear` (`{view: completed|trash}`; user-scoped bulk clear, returns `deleted_count`)
- `GET /v1/tasks/stream?task_ids=` (SSE `text/event-stream`; cookie sessions only — PAT clients keep polling)
- `POST /v1/admin/tasks/cancel-active` (admin; cancels every active task)
- `POST /v1/admin/tasks/{task_id}/recover` (admin; Docling recovery)
- `POST /v1/admin/tasks/{task_id}/recover/queue` (admin; queue Docling recovery)
- `POST /v1/admin/tasks/quarantine-submitting` (admin; quarantine stale submitting tasks)
- source and settings management endpoints under `/v1/*`

Task history is never auto-deleted: `completed` tasks persist until the user
clears completed history and `trash` tasks persist until the user restores,
deletes, or clears them. Task deletion never removes files, PostgreSQL text,
Qdrant vectors, or S3 objects. The retention/purge admin APIs
(`GET/PUT /v1/admin/tasks/maintenance`, `POST /v1/admin/tasks/purge`) are
removed; lease recovery, source cleanup, and Docling recovery/quarantine are
retained (see `docs/contracts/v0.17-migration.md`).

## Advanced SDK workflow

`context69-sdk` exposes the ergonomic facade plus the complete low-level
`client.raw()` transport (all 115 OpenAPI operations via the `OPERATIONS`
registry and `RawRequest`). Use `ensure_scope` once for group provisioning
and declared metadata indexes, then submit text, URL, file, or delete arrays
through `submit_text_batch`, `submit_url_batch`, `submit_file_batch`, or
`submit_delete_batch`. (The v0.15 `text_batch`/`url_batch`/`file_batch`/
`delete_batch` names remain as deprecated aliases in this release.) A
one-item array is the single-item form. Every submission returns a task
reference; use task status and item endpoints for progress and independent
failures. `list_tasks` requires `TaskListOptions` (`view`, `page` 1..=10_000,
`page_size` 1..=100; default `processing`/1/25) and `list_task_items` takes
`TaskItemsOptions` (`limit` 1..=100, default 100). `PUT /v1/settings/search`
takes `CanonicalUpdateSearchSettingsRequest` with `api_key: SecretPatch`
(`{op: keep|set|clear}`). Queue, lease, heartbeat, retry-attempt, URL
polling, and metadata-index workers remain server-side.

## Source file lifecycle

- Sources are retained by default. The canonical wire is `IngestOptions`
  (`source_policy: retain | release_after_processing`, default `retain`)
  carried in required `options` on every upload path (`multipart`,
  `FileBatch` base64, `prepare-upload`, URL import). The flattened v0.17
  fields (`metadata`/`translation`/`extraction` plus boolean
  `delete_source_after_processing`) are removed in v0.18: unknown fields
  (including those legacy keys) are rejected, and `metadata_json` must be
  an object map (`MetadataObject`). The policy is chosen once at upload
  and is never changed by a later dedup/reuse request. Stored worker
  payloads require canonical `options`; `rerun`/`retry` copy stored JSON
  verbatim and the worker rejects legacy rows on next claim (see
  `docs/contracts/v0.18-migration.md`).
- File rows hold only terminal states (`succeeded` / `failed`);
  `processing` is derived from active task items. The `pending` / `running`
  / `cancelled` file states are gone; stale rows were reclaimed to `failed`
  by `migrations/20260915000000_backfill_stale_file_status_400.sql`.
- With the opt-in set, the source object is released only after the ingest
  result is committed successfully. A crash or transient storage error leaves
  the release pending; the server retries it in the background.
- `POST .../library/files/{file_id}/release-source` releases one succeeded
  file's source on demand, requires the same group and maintainer role, is
  rejected while the file has an active processing task, and is idempotent.
  Sync-managed control files (`source.json` and synced records) are refused.
- Release deletes only the stored source object. The file record, processed
  full text, vectors, and original size are retained; `source_available`
  becomes `false`. Shared content-addressed objects are detached from the
  requesting file only and their bytes are deleted once no file or task item
  references them.
- Physical deletion is durable: the object row is kept until the bytes are
  confirmed deleted, and a storage failure or crash reschedules the deletion
  with backoff instead of leaving an unrecoverable orphan. The bytes of an
  object that is referenced again are never deleted.
- Release success means the logical release committed and physical deletion
  was queued, not that S3 deletion finished. Both manual and auto releases
  wake a shared cleanup dispatcher immediately after commit without waiting
  for S3; the dispatcher also drains once at startup and every 5 minutes as
  a fallback, and exits on shutdown.
- A released file cannot be reprocessed until its bytes are uploaded again; the
  upload restores the source and keeps the original upload-time policy.
- `context69-sdk` exposes `release_file_source(group_path, file_id)` and carries
  the upload policy through `IngestOptions` / `FileBatchItem.options`.

## Contract bounds and errors

- `GET /v1/tasks` requires typed `view` (`processing` | `completed` |
  `trash`); `trashed` is removed and rejected (`deny_unknown_fields`, so
  old `?trashed=` fails instead of silently changing meaning). `GET
  /v1/search/stream` takes the canonical cursor shape (no `page`).
  `GET /v1/tasks/stream?task_ids=` accepts at most 100 comma-separated
  UUIDs (blank means watch-all own tasks); malformed UUIDs or more than
  100 IDs return 400 before the SSE body starts.
  `page` is 1..=10_000 and `page_size`/`limit` are 1..=100 in both Rust
  validation and OpenAPI (`minimum: 1`). `Pagination` and
  `OffsetPagination` are both kept: exact totals without window signals
  serialize identically, windowed (`has_more`/`total_is_exact`) or
  `page > 10_000` responses require `Pagination`.
- Error responses carry a stable `code` (`ApiErrorCode`: `invalid_argument`,
  `not_found`, `conflict`, `unavailable`, `upstream_timeout`, ...). Match on
  `code`, not on message text.
- Full v0.15.19 to v0.16.0 breaking notes, including SDK renames and the
  deploy-together cutover, live in `docs/contracts/v0.16-migration.md`.
- Full v0.16.0 to v0.17.0 breaking notes (no auto-deletion, user clear
  endpoint, removed retention/purge admin APIs, retained lease/source
  cleanup/Docling recovery) live in `docs/contracts/v0.17-migration.md`.
- Full v0.17.1 to v0.18.0 breaking notes (terminal-only file states,
  `trashed` removal, options-only uploads with required
  `FileBatchItem.options`, pagination audit, personal-scope deprecations)
  live in `docs/contracts/v0.18-migration.md`.
- v0.18.0 to v0.19.0 is additive: `GET /v1/tasks/stream` (SSE,
  snapshot-then-deltas, polling retained as fallback) plus nginx SSE
  hardening and multi-replica notes in `docs/docker.md`. No breaking wire
  changes.

## Task streaming (SSE, v0.19)

- `GET /v1/tasks/stream` requires a cookie session (inherits task-route
  auth + workspace scope, `CurrentUser` filters to own tasks only);
  `EventSource` sends cookies via `withCredentials`, PAT clients cannot
  attach `Authorization` and stay on polling. Foreign or missing task IDs
  are silently skipped.
- Frames (`event:` names): `snapshot` (full states at subscribe time —
  explicit IDs in request order, watch-all covers the `processing` view
  first 100), then one `update` per watched task change (current full
  state), then `done` when every explicitly watched ID is terminal
  (`succeeded`/`failed`/`cancelled`; watch-all never sends `done`), or
  `error` (`{ "message" }` — resync via `GET /v1/tasks` and reconnect).
  Transport is PG NOTIFY (`task_events`: `task_id`/`item_id`/`status`/
  `updated_at`) fanning out per replica; best-effort, so every subscribe
  and every reconnect starts with a full sync (`GET /v1/tasks`) before
  applying deltas. `Last-Event-ID` is not replayed server-side.
- Nginx (`docker/nginx-context69.conf`) disables buffering/caching for
  `/v1/tasks/stream` with a 24h read/send timeout; multi-replica needs no
  stickiness (every replica LISTENs the same PG channel).

## Deprecated personal-scope library endpoints (v0.18, removal deferred past v0.19)

The 10 personal-scope `/v1/library/*` endpoints are deprecated in v0.18
(OpenAPI `deprecated: true`) and remain served in v0.19.0; removal is
deferred past v0.19 (no wire removal in this release). New clients must
use the group-scoped replacements; the frontend and SDK facade already do.

| Deprecated (v0.18) | Replacement |
| --- | --- |
| `GET /v1/library/tree` | `GET /v1/groups/by-path/{group_path}/library/tree` |
| `GET /v1/library/resources` | `GET /v1/groups/by-path/{group_path}/library/resources` |
| `POST /v1/library/folders` | `POST /v1/groups/by-path/{group_path}/library/folders` |
| `POST /v1/library/texts` | `POST /v1/groups/by-path/{group_path}/library/texts` |
| `POST /v1/library/folders/{folder_id}/move` | `POST /v1/groups/by-path/{group_path}/library/folders/{folder_id}/move` |
| `DELETE /v1/library/folders/{folder_id}` | `DELETE /v1/groups/by-path/{group_path}/library/folders/{folder_id}` |
| `POST /v1/library/files/upload` | `POST /v1/groups/by-path/{group_path}/library/files/upload` (+ `prepare-upload` for dedup) |
| `GET /v1/library/files/{file_id}` | `GET /v1/groups/by-path/{group_path}/library/files/{file_id}` |
| `POST /v1/library/files/{file_id}/move` | `POST /v1/groups/by-path/{group_path}/library/files/{file_id}/move` |
| `DELETE /v1/library/files/{file_id}` | `DELETE /v1/groups/by-path/{group_path}/library/files/{file_id}` |

Audit (issue 399 Task B3): no in-tree typed callers remain. The frontend
(`api-group-workspace.ts`) and SDK facade use only group-scoped routes; MCP
tools (`search`, `documents`, `sources`) never touch library paths. The
generic SDK `client.raw()` registry still lists the 10 operations, so pinned
external callers could invoke them by `operation_id` — deletion was deferred
past v0.18 and is still deferred in v0.19.0 (versions plus nginx plus docs
only, no wire removal in issue 405 Task E4).

## Authentication

- Browser sign-in uses `POST /v1/auth/login` and an HttpOnly signed session cookie backed by Valkey.
- Browser sessions expire after seven days of inactivity and are renewed by authenticated activity.
- Personal access tokens are opaque bearer tokens prefixed with `ctx_pat_`.
- PAT creation and revocation are only available to browser sessions, not to PAT callers.
- PAT scopes are coarse-grained: `search`, `workspace`, `library`, `sources`, `settings`, `admin`.

## OpenAPI

Export the OpenAPI document:

```bash
cargo run -- export-openapi
```

Generated output path:

```text
frontend/openapi/context69.openapi.json
```
