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

`context69-sdk` exposes only high-level operations. Use `ensure_scope` once for
group provisioning and declared metadata indexes, then submit text, URL, or
file arrays through the batch methods. A one-item array is the single-item
form. Every submission returns a task reference; use task status and item
endpoints for progress and independent failures. Queue, lease, heartbeat,
retry-attempt, URL polling, and metadata-index workers remain server-side.

## Source file lifecycle

- Sources are retained by default. Every upload path (`multipart`, `FileBatch`
  base64, `prepare-upload`, URL import) accepts a boolean
  `delete_source_after_processing` (default `false`). It is chosen once at
  upload and is never changed by a later dedup/reuse request.
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
  the upload flag through `FileBatchItem` / `UrlBatchItem`.

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
