-- Deterministic, bounded selection of legacy direct-path rows. The cursor on
-- (created_at, id) keeps restarts safe: rows migrated in earlier batches drop
-- out of the filter, and a failed row is simply not revisited until the next
-- invocation rescans from the beginning.
SELECT f.id,
       f.group_id,
       f.filename,
       f.size_bytes,
       f.sha256,
       f.storage_rel_path,
       f.created_at
FROM context69.library_files AS f
WHERE f.storage_object_id IS NULL
  AND ($1::timestamptz IS NULL OR (f.created_at, f.id) > ($1::timestamptz, $2::uuid))
ORDER BY f.created_at, f.id
LIMIT $3::bigint
