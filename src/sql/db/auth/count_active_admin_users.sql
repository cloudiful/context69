SELECT COUNT(*) AS "count!"
FROM context69.users
WHERE is_admin = true
  AND disabled_at IS NULL
