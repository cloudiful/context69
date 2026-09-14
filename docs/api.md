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
- `GET|PUT /v1/admin/tasks/maintenance` (admin; auto-cleanup settings and task statistics)
- `POST /v1/admin/tasks/cancel-active` (admin; cancels every active task)
- `POST /v1/admin/tasks/purge` (admin; purges expired or all terminal task history)
- source and settings management endpoints under `/v1/*`

## Advanced SDK workflow

`context69-sdk` exposes the ergonomic facade plus the complete low-level
`client.raw()` transport (all 116 OpenAPI operations via the `OPERATIONS`
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
  carried in `options` on every upload path (`multipart`, `FileBatch`
  base64, `prepare-upload`, URL import). The flattened v0.15 fields
  (`metadata`/`translation`/`extraction` plus boolean
  `delete_source_after_processing`, default `false`) are still accepted for
  compatibility and take effect only when `options` is absent; new clients
  send only `options`. The policy is chosen once at upload and is never
  changed by a later dedup/reuse request.
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
- A released file cannot be reprocessed until its bytes are uploaded again; the
  upload restores the source and keeps the original upload-time policy.
- `context69-sdk` exposes `release_file_source(group_path, file_id)` and carries
  the upload policy through `IngestOptions` / `FileBatchItem.options`.

## Contract bounds and errors

- `GET /v1/tasks` requires typed `view` (no `trashed`); `GET
  /v1/search/stream` takes the canonical cursor shape (no `page`).
  `page` is 1..=10_000 and `page_size`/`limit` are 1..=100 in both Rust
  validation and OpenAPI (`minimum: 1`).
- Error responses carry a stable `code` (`ApiErrorCode`: `invalid_argument`,
  `not_found`, `conflict`, `unavailable`, `upstream_timeout`, ...). Match on
  `code`, not on message text.
- Full v0.15.19 to v0.16.0 breaking notes, including SDK renames and the
  deploy-together cutover, live in `docs/contracts/v0.16-migration.md`.

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
