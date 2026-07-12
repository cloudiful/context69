SELECT id FROM context69.documents
WHERE group_id = $1
  AND ($2::text IS NULL OR source_key = $2)
  AND ($3::timestamptz IS NULL OR published_at >= $3)
  AND ($4::timestamptz IS NULL OR published_at <= $4)
ORDER BY id
