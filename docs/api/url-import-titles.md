# URL import task titles

[简体中文](url-import-titles.zh-CN.md)

URL imports accept an optional `title` that becomes the task subject in the
processing queue. The title is task metadata only: it never renames the imported
document, its library file, or their canonical titles.

## Direct URL import

`POST /v1/groups/by-path/{group_path}/library/files/import-url`

```json
{
  "url": "https://files.example.test/reports/2026-10-15-0099.PDF",
  "title": "Quarterly Filing",
  "options": {
    "metadata": {},
    "source_policy": "retain"
  }
}
```

`title`, `folder_id`, `filename`, and `media_type` are optional. `url` and
`options` are required, and unknown fields are rejected.

## URL batch

`POST /v1/groups/by-path/{group_path}/batch/url`

```json
{
  "items": [
    {
      "url": "https://files.example.test/reports/2026-10-15-0099.PDF",
      "title": "Quarterly Filing",
      "options": {
        "metadata": {},
        "source_policy": "retain"
      }
    }
  ]
}
```

The same item shape is accepted under `items` by `POST /v1/tasks` with
`group_path` and the `url_batch` task kind. Both routes persist the submitted
item in the existing task payload, so the document title projection returns the
title without a schema or projection change.

## Title handling

- Leading, trailing, and repeated whitespace is collapsed; internal single
  spaces remain.
- A missing, `null`, empty, or whitespace-only title is stored as absent and is
  omitted from the serialized task payload.
- A non-string title is rejected, as is any unknown field.
- Queue rows show the title first and the filename as secondary text when the
  two differ. Without a title, the existing filename, group, and source
  fallbacks apply.
- Existing URL, download, deduplication, and queue behavior is unchanged. The
  canonical document and file titles are not overwritten, and deduplication
  still reuses the same canonical document and file.

Callers must send `title` themselves; the server does not derive titles from PDF
metadata, filenames, or page content. Existing tasks are not retitled
retroactively, and no backfill is performed.
