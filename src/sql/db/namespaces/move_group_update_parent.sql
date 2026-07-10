UPDATE context69.groups
SET parent_group_id = $2,
    updated_at = now()
WHERE id = $1
