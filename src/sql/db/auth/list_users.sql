SELECT id, login_name, display_name, password_hash, is_admin, disabled_at, created_at, updated_at
FROM context69.users
WHERE (
       NULLIF(BTRIM($1::TEXT), '') IS NULL
    OR login_name ILIKE '%' || BTRIM($1::TEXT) || '%'
    OR display_name ILIKE '%' || BTRIM($1::TEXT) || '%'
)
ORDER BY
    CASE WHEN $2::TEXT = 'login_name' AND $3::TEXT = 'asc' THEN login_name END ASC NULLS LAST,
    CASE WHEN $2::TEXT = 'login_name' AND $3::TEXT = 'desc' THEN login_name END DESC NULLS LAST,
    CASE WHEN $2::TEXT = 'display_name' AND $3::TEXT = 'asc' THEN display_name END ASC NULLS LAST,
    CASE WHEN $2::TEXT = 'display_name' AND $3::TEXT = 'desc' THEN display_name END DESC NULLS LAST,
    CASE WHEN $2::TEXT = 'created_at' AND $3::TEXT = 'asc' THEN created_at END ASC NULLS LAST,
    CASE WHEN $2::TEXT = 'created_at' AND $3::TEXT = 'desc' THEN created_at END DESC NULLS LAST,
    login_name ASC,
    id ASC
LIMIT $4 OFFSET $5
