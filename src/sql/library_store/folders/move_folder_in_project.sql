UPDATE context69.library_folders
SET parent_id = $3, updated_at = now()
WHERE project_id = $1
  AND id = $2
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
