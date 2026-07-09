DELETE FROM context69.sync_runs
WHERE project_id = $1
  AND source_key = $2
