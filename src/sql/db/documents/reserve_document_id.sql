-- Reserve the next document id that is not a tombstoned entity.
--
-- The sequence never repeats a value, so a reserved id can only be a
-- tombstoned one when an explicit id was inserted ahead of the sequence. Such
-- an id must not be handed out again: the document is deleted, so a new
-- document with that id would be invisible to every search that still
-- excludes the tombstone. The statement therefore returns no row for a
-- tombstoned candidate, and the caller skips it within a bounded number of
-- attempts.
WITH candidate AS (
    SELECT nextval('context69.documents_id_seq'::regclass) AS id
)
SELECT candidate.id AS "id!"
FROM candidate
WHERE NOT EXISTS (
    SELECT 1
    FROM context69.library_deleted_entities
    WHERE document_id = candidate.id
)
