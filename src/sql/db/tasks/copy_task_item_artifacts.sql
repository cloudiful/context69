-- Copy the source item's artifact references onto a rerun item (issue #975 P4).
--
-- Only references that actually exist on the source item are copied, and only
-- when the source and the new item belong to the same non-NULL task group and
-- the registry object also belongs to that group. The rerun transaction already
-- holds the source task row lock, and the reference insert takes the object's
-- FOR KEY SHARE lock, so a concurrent artifact release cannot remove the bytes
-- between the payload snapshot and this copy.
INSERT INTO context69.task_item_artifacts (item_id, kind, storage_object_id, encoding)
SELECT $2, source.kind, source.storage_object_id, source.encoding
FROM context69.task_item_artifacts AS source
JOIN context69.task_items AS source_item ON source_item.id = source.item_id
JOIN context69.tasks AS source_task ON source_task.id = source_item.task_id
JOIN context69.task_items AS new_item ON new_item.id = $2
JOIN context69.tasks AS new_task ON new_task.id = new_item.task_id
JOIN context69.library_storage_objects AS object ON object.id = source.storage_object_id
WHERE source.item_id = $1
  AND new_item.task_id = $3
  AND new_task.group_id IS NOT NULL
  AND source_task.group_id = new_task.group_id
  AND object.group_id = new_task.group_id
ON CONFLICT (item_id, kind) DO NOTHING
