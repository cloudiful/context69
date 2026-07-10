UPDATE context69.groups
SET name = $2,
    visibility = $3,
    updated_at = now()
WHERE full_path = $1
