INSERT INTO context69.groups (group_key, name, visibility, kind, full_path)
VALUES ($1, $2, $3, 'shared', $4)
RETURNING id
