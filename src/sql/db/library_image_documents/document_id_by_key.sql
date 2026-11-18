-- Recheck the parent document inside the locked link transaction.
--
-- $1 group_id, $2 source_key, $3 external_id.
SELECT id
FROM context69.documents
WHERE group_id = $1
  AND source_key = $2
  AND external_id = $3
