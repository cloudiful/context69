UPDATE context69.projects
SET group_id = $2,
    updated_at = now()
WHERE id = $1
