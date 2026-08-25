-- Deterministic, bounded page of cleanup-eligible legacy old-key records.
-- Rows already marked deleted drop out of the filter, and the id cursor keeps
-- a permanently failing row from blocking later pages while remaining
-- restart-safe across invocations.
SELECT id,
       group_id,
       file_id,
       old_key,
       old_storage_backend
FROM context69.library_legacy_object_cleanup
WHERE deleted_at IS NULL
  AND cleanup_eligible_at <= now()
  AND ($2::bigint IS NULL OR id > $2::bigint)
ORDER BY id
LIMIT $1::bigint
