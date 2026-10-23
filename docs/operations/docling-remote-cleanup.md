# Docling remote tasks and cleanup

Docling settings expose two separate read surfaces and one explicit cleanup
wake-up. Both lists are admin-only.

## Remote tasks

`GET /v1/admin/docling/tasks` reads the saved Docling service. It is a read-only
view of a shared service, so it can include tasks submitted by other clients; it
does not describe Context69 ownership or a complete enumeration. Paging uses the
upstream `limit`/`offset` contract and `has_more`; there is no client-side total,
filter, or sort. The UI offers page sizes 20, 50, and 100 and shows only the
upstream `created_at` time.

## Local cleanup records

`GET /v1/admin/docling/cleanup` pages the persisted local cleanup outbox:

- `page` starts at 1 and `page_size` defaults to 20 with a maximum of 100.
- `status`, `outcome`, and `remote_task_id` are optional exact filters. The task
  ID is trimmed and must not be blank; there is no wildcard or substring match.
- The response contains the selected page and the real server `total`. Pages are
  ordered by `created_at` descending, then `id` descending.
- Rows expose the persisted `created_at`, `updated_at`, and optional
  `finished_at`, alongside the existing attempts, next attempt, and deadline.

Persisted status values are `pending`, `cancelling`, `awaiting_terminal`, `done`,
`unsupported`, and `failed`. Persisted outcome values are `success`,
`partial_success`, `failure`, `skipped`, `cancelled`, and `not_found`. A missing
outcome is absent, not an invented value.

The UI keeps filtering and page selection independent: changing a filter or page
size returns to page 1, Refresh keeps the current selection, and a late response
for an older query never replaces the current page.

## Cleanup wake-up

`POST /v1/admin/docling/cleanup` wakes cleanup for every eligible locally owned
stopped job, not only the filtered or visible page. It does not stop running work
or interrupt started conversions. Only pending, cancelling, or awaiting-terminal
rows within the cleanup deadline are woken; done, unsupported, failed, and
expired rows stay terminal. `not_found` records an observed remote disappearance,
not a confirmed cancellation.

The legacy `GET /v1/admin/docling/tasks` response still carries its deprecated
`cleanup` array with the previous wire shape so older callers keep working.
