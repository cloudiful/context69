UPDATE context69.users
SET password_hash = $2,
    updated_at = now()
WHERE login_name = $1
RETURNING id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
