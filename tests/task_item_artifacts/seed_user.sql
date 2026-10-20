INSERT INTO context69.users (login_name, display_name, password_hash)
VALUES ($1, 'Task Item Artifact Test', 'unused')
RETURNING id
