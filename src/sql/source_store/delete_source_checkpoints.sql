DELETE FROM context69.source_checkpoints
WHERE source_key = $1
  AND ($2::bigint IS NULL OR group_id = $2)
