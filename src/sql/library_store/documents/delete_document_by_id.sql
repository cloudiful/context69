-- One document row, removed on the connection of the delete transaction.
--
-- The delete of the document and the entity tombstone that excludes its
-- points commit together, so a document is never searchable without its
-- exclusion and never excluded without being gone.
DELETE FROM context69.documents
WHERE id = $1
