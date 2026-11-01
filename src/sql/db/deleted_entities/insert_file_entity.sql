-- One tombstone for a deleted library file.
--
-- The insert is the only writer of the row: a repeated delete of the same
-- file matches the unique partial index and inserts nothing, so the caller
-- knows that no new tombstone (and no new generation bump) is owed.
INSERT INTO context69.library_deleted_entities (file_id, group_id)
VALUES ($1, $2)
ON CONFLICT (file_id) WHERE file_id IS NOT NULL DO NOTHING
RETURNING id
