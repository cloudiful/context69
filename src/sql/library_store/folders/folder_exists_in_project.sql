-- Whether a project-scoped folder still exists on the caller transaction.
--
-- $1 group_id, $2 folder_id.
SELECT id
FROM context69.library_folders
WHERE group_id = $1
  AND id = $2
