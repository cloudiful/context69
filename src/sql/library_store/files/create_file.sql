WITH folder_scope AS (
    SELECT group_id, visibility
    FROM context69.library_folders
    WHERE id = $2
),
default_scope AS (
    SELECT id AS group_id, visibility
    FROM context69.groups
    WHERE full_path = 'public'
),
resolved_scope AS (
    SELECT group_id, visibility FROM folder_scope
    UNION ALL
    SELECT group_id, visibility FROM default_scope
    LIMIT 1
)
INSERT INTO context69.library_files (
    id,
    group_id,
    visibility,
    folder_id,
    external_id,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    storage_object_id,
    ingest_status
)
SELECT
    $1,
    rs.group_id,
    rs.visibility,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    $9,
    'pending'
FROM resolved_scope rs
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
