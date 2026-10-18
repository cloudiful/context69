INSERT INTO context69.users (login_name, display_name, password_hash)
VALUES ($1, $2, $3)
RETURNING id
