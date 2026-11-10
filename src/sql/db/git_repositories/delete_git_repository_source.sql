-- Removes one repository source after its generations and vectors are gone.
-- The row's generations, files, chunks, webhook registration, and active
-- pointer cascade; webhook deliveries keep their history with a null source.
--
-- $1 group_id, $2 repository_key.
DELETE FROM context69.git_repository_sources
WHERE group_id = $1
  AND repository_key = $2
RETURNING repository_key
