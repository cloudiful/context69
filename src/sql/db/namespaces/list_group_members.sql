SELECT
    u.id AS "user_id!",
    u.login_name,
    u.display_name,
    gm.role
FROM context69.group_memberships gm
JOIN context69.users u ON u.id = gm.user_id
JOIN context69.groups g ON g.id = gm.group_id
WHERE g.full_path = $1
  AND (
       NULLIF(BTRIM($2::TEXT), '') IS NULL
    OR u.login_name ILIKE '%' || BTRIM($2::TEXT) || '%'
    OR u.display_name ILIKE '%' || BTRIM($2::TEXT) || '%'
  )
ORDER BY
    CASE WHEN $5::TEXT = 'login_name' AND $6::TEXT = 'asc' THEN u.login_name END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'login_name' AND $6::TEXT = 'desc' THEN u.login_name END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'display_name' AND $6::TEXT = 'asc' THEN u.display_name END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'display_name' AND $6::TEXT = 'desc' THEN u.display_name END DESC NULLS LAST,
    CASE WHEN $5::TEXT = 'role' AND $6::TEXT = 'asc'
        THEN CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END
    END ASC NULLS LAST,
    CASE WHEN $5::TEXT = 'role' AND $6::TEXT = 'desc'
        THEN CASE gm.role WHEN 'owner' THEN 3 WHEN 'maintainer' THEN 2 ELSE 1 END
    END DESC NULLS LAST,
    u.login_name ASC,
    u.id ASC
LIMIT $3 OFFSET $4
