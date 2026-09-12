-- Deliberately released files may not be reprocessed: their source object is
-- gone and re-running ingest would fail or silently operate on nothing.
SELECT count(*)::bigint AS "count!"
FROM context69.library_files
WHERE group_id = $1
  AND id = ANY($2)
  AND source_released_at IS NOT NULL
