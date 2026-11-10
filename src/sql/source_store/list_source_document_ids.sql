SELECT id AS "id!"
FROM context69.documents
WHERE source_key = $1
  AND ($2::bigint IS NULL OR group_id = $2)
ORDER BY id
