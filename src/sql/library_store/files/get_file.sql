SELECT
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    folder_id,
    external_id,
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
FROM context69.library_files
WHERE id = $1
