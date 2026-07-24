SELECT count(*)::bigint
FROM context69.tasks
WHERE user_id = $1
  AND ($2::text IS NULL OR kind = $2)
  AND ($3::text IS NULL OR status = $3)
