UPDATE context69.library_files
SET
    folder_id = $3,
    external_id = $4,
    filename = $5,
    media_type = $6,
    size_bytes = $7,
    sha256 = $8,
    storage_rel_path = $9,
    ingest_status = 'pending',
    error_message = NULL,
    ingested_at = NULL,
    updated_at = now()
WHERE project_id = $1
  AND id = $2
RETURNING
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
