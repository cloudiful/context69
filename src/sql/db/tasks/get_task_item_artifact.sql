-- Load the existence of one artifact reference joined with its registry object
-- (issue #975 P3, repaired in P8).
--
-- This query answers "is there a reference row for this item/kind", never "is
-- there a usable reference": group, backend, encoding and registry identity are
-- projected for validation in Rust and are deliberately NOT used as filters.
-- Predicating them away here made an existing but unusable checkpoint look
-- absent, which let task stages reconvert or fall back to inline bytes instead
-- of reporting the bad reference.
SELECT artifact.item_id,
       artifact.kind,
       artifact.encoding,
       artifact.storage_object_id,
       task.group_id AS task_group_id,
       object.group_id AS object_group_id,
       object.sha256,
       object.size_bytes,
       object.object_key,
       object.storage_backend
FROM context69.task_item_artifacts AS artifact
JOIN context69.task_items AS item
  ON item.id = artifact.item_id
JOIN context69.tasks AS task
  ON task.id = item.task_id
LEFT JOIN context69.library_storage_objects AS object
  ON object.id = artifact.storage_object_id
WHERE artifact.item_id = $1
  AND artifact.kind = $2
