-- Deterministic, bounded selection of legacy direct-path rows whose
-- recorded source object is a candidate for startup missing-source
-- cleanup: still pointing at a legacy direct-path key
-- (`storage_object_id IS NULL`), in a terminal ingest state
-- (`succeeded` or `failed`), and old enough that the active storage key
-- has had a chance to settle (`created_at < now() - grace_hours`).
-- The `(created_at, id)` cursor keeps restarts safe: a row concurrently
-- linked, ingested again, or deleted drops out of the filter on the
-- next pass.
SELECT f.id,
       f.group_id,
       f.filename,
       f.size_bytes,
       f.sha256,
       f.storage_rel_path,
       f.ingest_status,
       f.created_at
FROM context69.library_files AS f
WHERE f.storage_object_id IS NULL
  AND f.ingest_status IN ('succeeded', 'failed')
  AND f.created_at < (now() - ($1::bigint * INTERVAL '1 hour'))
  AND ($2::timestamptz IS NULL OR (f.created_at, f.id) > ($2::timestamptz, $3::uuid))
ORDER BY f.created_at, f.id
LIMIT $4::bigint
