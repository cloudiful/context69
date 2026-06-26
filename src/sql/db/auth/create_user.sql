INSERT INTO context69.users (
    login_name,
    display_name,
    password_hash,
    is_admin
)
VALUES ($1, $2, $3, $4)
RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
