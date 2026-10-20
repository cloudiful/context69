-- Known pre-artifact recovery profile only (issue #975 P9).
--
-- Artifact-free twin of `clear_expired_staging_with_file_reference.sql`.
-- Selected only while `Database::pending_upgrade()` reports the validated
-- pinned admission, where the migration ledger proves
-- context69.task_item_artifacts does not exist and no artifact row can exist.
-- It is never chosen from a query error, a relation probe or configuration.

WITH candidates AS (
    SELECT object.id
    FROM context69.library_storage_objects AS object
    WHERE object.staging_lease_until IS NOT NULL
      AND object.staging_lease_until < $1
      AND EXISTS (
          SELECT 1
          FROM context69.library_files AS file
          WHERE file.storage_object_id = object.id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items AS item
          WHERE item.input_storage_object_id = object.id
      )
    ORDER BY object.updated_at
    LIMIT $2
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.library_storage_objects AS object
SET staging_lease_until = NULL,
    updated_at = now()
FROM candidates
WHERE object.id = candidates.id
