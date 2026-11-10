-- One tombstone for an explicitly deleted document.
--
-- The insert is the only writer of the row: a repeated delete of the same
-- document matches the unique partial index and inserts nothing, so the caller
-- knows that no new tombstone (and no new generation bump) is owed. The
-- recorded physical target is immutable: a later delete never overwrites it
-- and never fills a legacy NULL origin from a later current target.
INSERT INTO context69.library_deleted_entities (
    document_id, group_id, target_endpoint, target_collection
)
VALUES ($1, $2, $3, $4)
ON CONFLICT (document_id) WHERE document_id IS NOT NULL DO NOTHING
RETURNING id
