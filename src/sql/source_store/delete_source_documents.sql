DELETE FROM context69.documents
WHERE source_key = $1
  AND ($2::bigint IS NULL OR group_id = $2)
