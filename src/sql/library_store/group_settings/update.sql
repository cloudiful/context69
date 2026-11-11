UPDATE context69.groups
SET
    default_source_policy = $2,
    updated_at = now()
WHERE id = $1
RETURNING default_source_policy
