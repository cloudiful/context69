DELETE FROM context69.source_checkpoints
WHERE group_id = $1
  AND source_key = $2
