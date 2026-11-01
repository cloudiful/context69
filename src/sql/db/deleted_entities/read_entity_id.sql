-- The tombstone row of one deleted entity.
--
-- The lookup is the identity of the pending cleanup: the row that carries the
-- `purged_at` observation of the physical purge. Exactly one of the two ids
-- is set, so exactly one tombstone can match.
SELECT id
FROM context69.library_deleted_entities
WHERE ($1::uuid IS NOT NULL AND file_id = $1)
   OR ($2::bigint IS NOT NULL AND document_id = $2)
ORDER BY id
LIMIT 1
