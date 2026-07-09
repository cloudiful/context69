DELETE FROM context69.source_checkpoints
WHERE project_id = $1
  AND source_key = $2
