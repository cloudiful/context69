INSERT INTO context69.project_memberships (project_id, user_id, role)
VALUES ($1, $2, 'owner')
ON CONFLICT (project_id, user_id) DO NOTHING
