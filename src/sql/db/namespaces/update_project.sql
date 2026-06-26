UPDATE context69.projects
SET name = $3,
    visibility = $4,
    updated_at = now()
WHERE group_id = $1 AND project_key = $2
