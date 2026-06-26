SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
FROM context69.users
WHERE disabled_at IS NULL
  AND (
    lower(login_name) LIKE $1
    OR lower(display_name) LIKE $1
  )
ORDER BY login_name
LIMIT $2
