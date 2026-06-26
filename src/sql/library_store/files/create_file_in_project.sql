WITH folder_scope AS (
    SELECT group_id, project_id, visibility
    FROM context69.library_folders
    WHERE id = $2
      AND project_id = $9
),
project_scope AS (
    SELECT p.group_id, p.id AS project_id, p.visibility
    FROM context69.projects p
    WHERE p.id = $9
),
resolved_scope AS (
    SELECT group_id, project_id, visibility FROM folder_scope
    UNION ALL
    SELECT group_id, project_id, visibility FROM project_scope
    LIMIT 1
)
INSERT INTO context69.library_files (
    id,
    group_id,
    project_id,
    visibility,
    folder_id,
    external_id,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    ingest_status
)
SELECT
    $1,
    rs.group_id,
    rs.project_id,
    rs.visibility,
    $2,
    $3,
    $4,
    $5,
    $6,
    $7,
    $8,
    'pending'
FROM resolved_scope rs
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
