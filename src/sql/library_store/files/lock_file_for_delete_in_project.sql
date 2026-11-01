-- The library file row a project-scoped delete locks.
--
-- The group is part of the lock, so a delete of another project never locks
-- (and never deletes) a file it does not own.
SELECT
    id,
    group_id,
    storage_rel_path,
    storage_object_id,
    source_released_at,
    sha256
FROM context69.library_files
WHERE id = $1
  AND group_id = $2
FOR UPDATE
