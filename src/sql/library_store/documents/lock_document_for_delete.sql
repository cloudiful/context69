-- The document row an explicit document delete locks.
--
-- The group is part of the lock, so a delete never touches a document of
-- another group.
SELECT id, group_id, source_key, external_id
FROM context69.documents
WHERE id = $1
  AND group_id = $2
FOR UPDATE
