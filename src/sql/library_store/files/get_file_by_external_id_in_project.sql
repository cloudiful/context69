SELECT
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    project_id,
    (SELECT project_key FROM context69.projects WHERE id = project_id) AS "project_key!",
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
WHERE project_id = $1
  AND external_id = $2
