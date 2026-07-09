UPDATE context69.sync_runs
SET source_key = $3,
    updated_at = now()
WHERE project_id = $1
  AND source_key = $2
