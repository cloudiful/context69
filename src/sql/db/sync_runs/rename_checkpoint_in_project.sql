UPDATE context69.source_checkpoints
SET source_key = $3,
    updated_at = now()
WHERE project_id = $1
  AND source_key = $2
