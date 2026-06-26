INSERT INTO context69.project_memberships (project_id, user_id, role)
VALUES ($1, $2, $3)
ON CONFLICT (project_id, user_id) DO UPDATE
SET role = EXCLUDED.role,
    updated_at = now()
