-- The filenames occupying one group folder, on the caller connection.
--
-- $1 group_id, $2 folder_id (NULL is the project root).
SELECT filename
FROM context69.library_files
WHERE group_id = $1
  AND folder_id IS NOT DISTINCT FROM $2
ORDER BY filename, id
