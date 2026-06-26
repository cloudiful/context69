WITH project_scope AS (
    SELECT p.group_id, p.id AS project_id, p.visibility
    FROM context69.projects p
    WHERE p.id = $4
),
parent_scope AS (
    SELECT group_id, project_id, visibility
    FROM context69.library_folders
    WHERE id = $2 AND project_id = $4
),
resolved_scope AS (
    SELECT group_id, project_id, visibility FROM parent_scope
    UNION ALL
    SELECT group_id, project_id, visibility FROM project_scope
    LIMIT 1
)
INSERT INTO context69.library_folders (
    id,
    group_id,
    project_id,
    visibility,
    parent_id,
    name
)
SELECT $1, rs.group_id, rs.project_id, rs.visibility, $2, $3
FROM resolved_scope rs
RETURNING
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    project_id,
    (SELECT project_key FROM context69.projects WHERE id = project_id) AS "project_key!",
    visibility,
    id,
    parent_id,
    name,
    created_at,
    updated_at
