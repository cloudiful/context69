-- Known pre-artifact recovery profile only (issue #975 P9).
--
-- Artifact-free twin of `get_staged_storage_object_for_update.sql`.
-- Selected only while `Database::pending_upgrade()` reports the validated
-- pinned admission, where the migration ledger proves
-- context69.task_item_artifacts does not exist and no artifact row can exist.
-- It is never chosen from a query error, a relation probe or configuration.

-- Lock a releasable staged object on the known pre-artifact profile.
--
-- Only the file and task-input referrers exist in this profile, so those are
-- the complete predicates here. A caller must still re-count references on the
-- same connection after this lock is acquired and before any physical delete:
-- these predicates are evaluated before the lock wait, so a reference that
-- commits while the lock is pending would not be visible in this snapshot.
SELECT id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
FROM context69.library_storage_objects AS object
WHERE object.id = $1
  AND object.staging_lease_until IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_files AS file
      WHERE file.storage_object_id = object.id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items AS item
      WHERE item.input_storage_object_id = object.id
  )
FOR UPDATE
