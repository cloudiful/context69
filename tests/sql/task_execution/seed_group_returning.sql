INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1, $2, 'public', 'shared', $3) RETURNING id, group_key, name, full_path, created_at, updated_at
