INSERT INTO context69.library_url_import_jobs (
    id, group_id, visibility, folder_id, source_url, dedupe_key,
    requested_filename, requested_media_type, external_id, source_uri,
    published_at, metadata_json, metadata_provided
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
ON CONFLICT (group_id, dedupe_key)
    WHERE status IN ('queued', 'downloading', 'ingesting')
DO UPDATE SET updated_at = context69.library_url_import_jobs.updated_at
RETURNING *
