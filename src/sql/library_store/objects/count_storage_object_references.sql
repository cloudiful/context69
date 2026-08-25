-- Number of live references to a storage object across library_files and
-- staged task inputs. Used to verify whether a guarded unreferenced-object
-- deletion actually removed the object or skipped it because it is shared.
SELECT COALESCE((
    SELECT count(*)
    FROM context69.library_files AS file
    WHERE file.storage_object_id = $1
) + (
    SELECT count(*)
    FROM context69.task_items AS item
    WHERE item.input_storage_object_id = $1
), 0)::bigint AS "references!"
