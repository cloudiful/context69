SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.group_memberships gm
JOIN context69.users u ON u.id = gm.user_id
JOIN context69.groups g ON g.id = gm.group_id
WHERE g.full_path = $1
  AND (
       NULLIF(BTRIM($2::TEXT), '') IS NULL
    OR u.login_name ILIKE '%' || BTRIM($2::TEXT) || '%'
    OR u.display_name ILIKE '%' || BTRIM($2::TEXT) || '%'
  )
