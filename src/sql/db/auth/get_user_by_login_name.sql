SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
FROM context69.users
WHERE login_name = $1
