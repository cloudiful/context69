-- Legacy direct-path identity guard: another live library_files row still
-- using the same storage_rel_path must block the physical delete. The
-- deliberately released row itself is excluded by `source_released_at`.
SELECT count(*)::bigint AS "count!"
FROM context69.library_files
WHERE storage_rel_path = $1
  AND source_released_at IS NULL
