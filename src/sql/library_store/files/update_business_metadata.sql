UPDATE context69.library_files
SET external_id = $3,
    source_uri = $4,
    published_at = $5,
    metadata_json = $6,
    updated_at = now()
WHERE group_id = $1 AND id = $2
RETURNING
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    folder_id,
    external_id,
    source_uri,
    published_at,
    metadata_json,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    ingest_status,
    error_message,
    created_at,
    updated_at,
    ingested_at
