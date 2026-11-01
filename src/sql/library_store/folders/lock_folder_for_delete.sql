-- The library folder row a delete locks.
--
-- The folder delete locks the registry epoch first and this row second, so a
-- folder delete never holds a folder lock while it waits for the epoch.
SELECT id, group_id, parent_id
FROM context69.library_folders
WHERE id = $1
FOR UPDATE
