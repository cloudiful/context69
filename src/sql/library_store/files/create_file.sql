WITH folder_scope AS (
    SELECT group_id, project_id, visibility
    FROM context69.library_folders
    WHERE id = $2
),
default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id, 'public'::text AS visibility
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
),
resolved_scope AS (
    SELECT group_id, project_id, visibility FROM folder_scope
    UNION ALL
    SELECT group_id, project_id, visibility FROM default_scope
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
