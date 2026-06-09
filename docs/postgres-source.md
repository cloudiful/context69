# PostgreSQL Source Requirements

The built-in `postgres_sql` connector expects the source query to return a normalized record shape.

## Required Columns

- `external_id`
- `title`
- `body_text`
- `source_uri`
- `updated_at`
- `metadata_json`

## Optional Columns

- `summary`
- `published_at`

## Notes

- `metadata_json` should be valid JSON
- `updated_at` should reflect source freshness for change detection
- the query should return stable IDs so document versions can be tracked correctly
