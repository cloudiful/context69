INSERT INTO context69.group_memberships (group_id, user_id, role)
VALUES ($1, $2, $3)
ON CONFLICT (group_id, user_id) DO UPDATE
SET role = EXCLUDED.role,
    updated_at = now()
