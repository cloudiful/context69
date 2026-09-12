SELECT count(*)::bigint AS "count!"
FROM context69.library_files
WHERE storage_rel_path = $1
  AND id <> $2
  AND source_released_at IS NULL
