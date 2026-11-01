-- The library folder row a project-scoped delete locks.
--
-- The group is part of the lock, so a delete of another project never locks
-- (and never deletes) a folder it does not own.
SELECT id, group_id, parent_id
FROM context69.library_folders
WHERE id = $1
  AND group_id = $2
FOR UPDATE
