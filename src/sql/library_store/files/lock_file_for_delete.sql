-- The library file row a delete locks.
--
-- The delete transaction locks the registry epoch first and this row second,
-- so the epoch is always the first lock of the transaction and the delete
-- backstop trigger of the file row finds the epoch lock already held. The
-- columns are the identity the delete needs: the group of the tombstone audit
-- value, the storage reference of the source cleanup intent and the release
-- state that keeps a deliberately released source out of the cleanup.
SELECT
    id,
    group_id,
    storage_rel_path,
    storage_object_id,
    source_released_at,
    sha256
FROM context69.library_files
WHERE id = $1
FOR UPDATE
