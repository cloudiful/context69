-- Remove a drained retirement row, only while the exact lease still owns it.
DELETE FROM context69.vector_collection_retirement
WHERE id = $1
  AND lease_token = $2
