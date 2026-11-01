-- The tombstones whose points were not observed as physically gone yet.
--
-- The list is bounded by the caller and ordered by the deletion time, so a
-- purge worker observes the oldest tombstones first. A purged row stays in
-- this table: `purged_at` records the physical observation, it never removes
-- the exclusion.
SELECT id, file_id, document_id, group_id, deleted_at, purged_at
FROM context69.library_deleted_entities
WHERE purged_at IS NULL
ORDER BY deleted_at, id
LIMIT $1
