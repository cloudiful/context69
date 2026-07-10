DELETE FROM context69.sync_runs
WHERE group_id = $1
  AND source_key = $2
