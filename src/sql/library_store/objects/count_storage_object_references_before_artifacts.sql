-- Known pre-artifact recovery profile only (issue #975 P9).
--
-- Artifact-free twin of `count_storage_object_references.sql`.
-- Selected only while `Database::pending_upgrade()` reports the validated
-- pinned admission, where the migration ledger proves
-- context69.task_item_artifacts does not exist and no artifact row can exist.
-- It is never chosen from a query error, a relation probe or configuration.

-- Number of live references to a storage object across library_files and
-- staged task inputs. The artifact relation does not exist in this profile, so
-- these two referrers are the complete set.
SELECT COALESCE((
    SELECT count(*)
    FROM context69.library_files AS file
    WHERE file.storage_object_id = $1
) + (
    SELECT count(*)
    FROM context69.task_items AS item
    WHERE item.input_storage_object_id = $1
), 0)::bigint AS "references!"
