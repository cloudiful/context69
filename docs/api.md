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
- source and settings management endpoints under `/v1/*`

Task history is never auto-deleted: `completed` tasks persist until the user
clears completed history and `trash` tasks persist until the user restores,
deletes, or clears them. Task deletion never removes files, PostgreSQL text,
Qdrant vectors, or S3 objects. The retention/purge admin APIs
(`GET/PUT /v1/admin/tasks/maintenance`, `POST /v1/admin/tasks/purge`) are
removed; lease recovery, source cleanup, and Docling recovery are
retained (see `docs/contracts/v0.17-migration.md`). Stale `submitting`
quarantine (`quarantine-submitting`) was removed in issue 446 P3: no
frontend entry remained and uncertain `submitting` rows now surface as a
409 conflict for manual handling.

## Advanced SDK workflow

`context69-sdk` exposes the ergonomic facade plus the complete low-level
`client.raw()` transport (all 114 OpenAPI operations via the `OPERATIONS`
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

## Git repository file manifest

- `GET /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/files`
  returns one bounded page of the path manifest the repository's active index
  generation serves, ordered by path. It takes the shared cursor query
  (`limit` 1..=100, default 50, plus `cursor`) and answers with
  `GitRepositoryFileListResponse`: the manifest entries plus the serving
  generation's key, number, ref, and pinned commit, the repository's index
  status and target/indexed commit checkpoint, and the generation's coverage
  counts.
- Continuation is the shared cursor shape: `next_cursor` is present exactly when
  `has_more` is true, and passing it back as `cursor` returns the next page. A
  continuation this API did not issue, or a `limit` outside the bound, is a
  `400 invalid_argument`.
- The page is metadata only: file bytes, chunk text, secret references, and
  provider connection state never cross it. An unknown or foreign repository is
  a `404`, and a repository with no active ready generation is a `409` rather
  than an empty page. Read access needs the `sources` scope and at least Viewer
  in the group.
- `GET /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file?path=<path>`
  answers the same questions for one exact path of that same generation. The
  `path` parameter is a query parameter because repository paths contain `/`; it
  is bounded at 512 characters and validated with the same path rules the stored
  entry passed, so a traversal or control-byte value is a `400 invalid_argument`
  that is refused before any lookup. The response
  (`GitRepositoryFileDetailResponse`) is one `GitRepositoryFile` entry plus the
  same provenance and coverage the manifest page reports.
- Both reads resolve the group, the Viewer floor, and the serving generation
  identically, so a detail read and the page it came from always describe the
  same commit. A path the serving generation does not hold returns exactly the
  unknown-repository response — same status, same body — so the two reads cannot
  be used to probe which repository keys exist or which paths another group
  indexed. Neither read returns file bytes, chunk text, a line or
  byte range, a provider blob id, or any credential, secret reference, or
  connection state.
- `GET /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file/content?path=<path>&start_line=<n>&end_line=<n>&cursor=<token>`
  is the one content egress path. It returns the stored UTF-8 text of one
  inclusive line window, verbatim: CRLF endings, trailing whitespace, and a
  missing final newline are preserved, so a caller can quote the text or
  concatenate continuation pages in order. The window is at most 400 lines, the
  returned text at most 64 KiB, and a `limit`-free `cursor` is the server-issued
  offset of the next matching chunk; `next_cursor` is present exactly when
  `has_more` is true. The response
  (`GitRepositoryFileContentResponse`) carries that text with its exact
  `byte_count`, the same manifest entry, and the same provenance and coverage
  the other two reads report.
- A zero, negative, reversed, or over-wide window and a continuation this API did
  not issue are `400 invalid_argument`. The content read never selects the raw
  acquisition blob or a provider blob id: only stored, line-anchored chunk text
  of the requested window is returned, and nothing outside that window appears
  in it.
- `GET /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/code-search?query=<term>&path_prefix=<prefix>&language=<token>&limit=<n>`
  is the one search read. It searches the serving generation with the same
  group-scoped lexical matching the `search_code` MCP tool uses — whole-term,
  case-insensitive, identifier-aware, with a case-sensitive Git path prefix and
  the classified `language` token as optional filters — and answers from stored
  chunks only. `query` is required and at most 200 characters, `path_prefix` is
  an optional repository-relative prefix of at most 512 characters, `language`
  is an optional lowercase token of at most 32 characters, and `limit` is
  `1..=50` with a default of 20. A blank or over-long term, an absolute,
  traversing, or control-byte prefix, a non-classified language, and an
  out-of-range limit are `400 invalid_argument`, and the refusal names the rule,
  never the submitted value. A `%` or `_` in the term stays a literal character,
  and only the term's surrounding whitespace is dropped. The prefix is matched
  literally and case-sensitively against the stored repository-relative path, so
  `src` also matches `srcfoo.rs`; one trailing `/` is dropped so a directory
  prefix stays expressible, and an absent, empty, or separator-only prefix
  narrows nothing.
- The search response (`GitCodeSearchResponse`) names the serving repository,
  generation, ref, pinned commit, index status, commit checkpoint, and
  file/excluded/byte coverage, then returns at most `limit` `GitCodeSearchHit`
  entries in score, path, and chunk order. Each hit carries the repository,
  generation, ref, commit, visibility, file, path, language, chunk, inclusive
  line range, score, match kind, and the verbatim stored chunk text. `truncated`
  is true exactly when the storage layer held more matches than the page
  returns; there is no cursor, so a caller that needs more narrows its filters.
  An empty valid search is an empty hit list with the same provenance and
  `truncated = false`. A repository without a serving generation is `409`, and
  an unknown or foreign repository is the same bounded `404` the other Git reads
  return. The search never returns a raw acquisition blob, a provider blob id, a
  secret reference, a credential, or connection state.
- `GET /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/diff?from_generation=<uuid>&to_generation=<uuid>&limit=<n>&cursor=<token>`
  compares two stored index generations and returns metadata only. Both
  generation parameters are optional: an omitted `from_generation` resolves to the
  comparable generation covering the source's current `indexed_commit_sha` — which
  is the superseded snapshot, because indexing a newer commit activates a newer
  generation — and an omitted `to_generation` to the repository's active ready
  generation, so the common case is "what changed since the last index". An
  explicit key is never trusted as given: it is resolved through the same group-
  and repository-confined read and must name a completed generation of this
  repository, and a key that does not is indistinguishable from a key that names
  nothing.
- The response (`GitRepositoryFileDiffResponse`) names both generations with
  their numbers, refs, and pinned commits, reports the source's index status and
  commit checkpoint plus both coverage envelopes, and returns at most `limit`
  changes in repository-path order with the shared cursor continuation. Each
  change carries its `path`, its `change_kind` (`added` | `modified` | `deleted`),
  and the safe manifest metadata of each side that exists (`file_key`, `language`,
  `byte_count`, `line_count`). A path whose stored bytes are identical in both
  generations is omitted, so an empty list is the truthful answer for two
  generations that hold the same bytes at the same paths.
- The comparison is decided in the database: the two manifests are joined on the
  repository-relative path and their stored provider content addresses are
  compared there, so no provider blob id, raw blob, chunk text, line- or
  content-level diff, symbol, credential, secret reference, or connection state
  exists anywhere on this path. `modified` therefore means the stored content
  address differs, not that the text differs. A comparable generation is a
  completed one — the active one, or one a newer snapshot superseded, which is the
  state the checkpoint's generation is in as soon as a newer commit is indexed. A
  generation that is still building or has failed is refused even when it holds
  manifest rows, an unknown or foreign repository is the same bounded `404` the
  other Git reads return, a generation that cannot be resolved is `409`, and a
  pair whose `from_generation` is newer than its `to_generation` is `409` rather
  than an empty comparison. An identical pair compares to an empty page.
- `DELETE /v1/groups/by-path/{group_path}/git-connections/{connection_key}` takes
  an existing group-owned provider connection out of service. It needs the same
  Maintainer role as creating a connection, so a Viewer or a non-member is refused
  with `403` before the connection is even read, and it answers with the existing
  `GitProviderConnection` projection: the same fields a create returns, with
  `disabled: true` read back from storage after the update.
- The action is idempotent by construction: the stored `COALESCE(disabled_at,
  now())` sets the timestamp once, so a repeated disable is a second `200` with
  the same projection rather than a conflict, and the *first* disable timestamp is
  the one that survives. Neither lifecycle action offers a delete, metadata edit,
  or credential rotation, and the route never opens the secret store, reads a
  credential, signing value, or App private key, or calls a provider — the
  projection reports only *whether* a read credential and a webhook secret are
  configured. An unknown or foreign connection key is the same bounded `404` the
  other connection routes return, so the route cannot be used to probe which keys
  exist.
- `POST /v1/groups/by-path/{group_path}/git-connections/{connection_key}/enable`
  takes a connection back into service, under the same Maintainer floor, with no
  request body. It answers with the same `GitProviderConnection` projection, with
  `disabled: false` read back from storage after the update.
- The enable statement clears only `disabled_at` and stamps `updated_at`. The
  stored read-credential, App-private-key, and webhook-signing-secret references
  are left exactly as they were, and nothing on the path reaches the internal
  secret store, so an enable/disable cycle can neither rotate nor lose a
  credential. It deliberately does not require the connection to be disabled, so
  enabling an already-enabled connection — or repeating an enable — is the same
  successful `200` rather than a conflict, and an unknown or foreign key is the
  same bounded `404` the disable path returns. Deletion, metadata edits,
  credential rotation, hook setup, acquisition, and MCP exposure remain outside
  both lifecycle actions.
- `PUT /v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook`
  registers the repository's webhook and answers `201` with the same
  `GitWebhookRegistration` projection the existing read returns. It is
  create-only: it never updates, rotates, deactivates, or deletes a registration,
  never creates the hook at the provider, never processes a delivery, and has no
  MCP tool. The group and its Maintainer floor are checked before any repository
  or secret work, so a Viewer or non-member is refused with `403` and an unknown
  or foreign repository is the same bounded `404` the other Git routes return.
- The body is `GitWebhookRegistrationRequest` — the `provider` (which must match
  the repository's own, or the request is a `409` before anything is written), a
  provider-issued `external_hook_id` bounded to the ingress limit of 1..=255 UTF-8
  bytes, the existing `ownership`, and an optional plain `signing_secret`. The
  bound is bytes, not characters, so the create and the signed ingress accept and
  refuse exactly the same ids.
  It carries no `active` flag and no store key or reference field: a non-blank
  secret makes the registration active, an omitted one stores it inactive, and the
  signed ingress rejects an inactive registration as it already does. A blank or
  whitespace-only hook id or supplied secret is a `400`, and the value is sealed
  untrimmed, because a provider signs over the exact configured secret.
- Creation is serialized by a transaction-scoped advisory lock keyed by the owning
  group and repository. Under that lock the group-owned registration is pre-checked
  — so a duplicate is a `409` with no secret-store write at all — then a supplied
  secret is sealed into the unified store under its own `git_webhook.signing_secret`
  purpose at the key name derived from the repository record, never from the
  caller's hook id, and the create-only insert writes the row and that reference
  together. The insert has no conflict clause, so a second registration on one
  repository and a hook identity another repository already claims are both the
  same bounded `409` and never overwrite or repoint an existing registration. A
  failed seal leaves no row; the only residue of a lost race is the store's own
  reclaimable sealed orphan. The response carries presence only — never a signing
  secret, ciphertext, store key, derived key, or provider state — and the value is
  never echoed, logged, or returned.

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
