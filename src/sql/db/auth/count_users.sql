SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.users
WHERE (
       NULLIF(BTRIM($1::TEXT), '') IS NULL
    OR login_name ILIKE '%' || BTRIM($1::TEXT) || '%'
    OR display_name ILIKE '%' || BTRIM($1::TEXT) || '%'
  )
