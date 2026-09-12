-- Identity lock for a source-object cleanup. Holding FOR UPDATE on the object
-- row until the transaction ends conflicts with the FOR KEY SHARE lock every
-- new library_files.storage_object_id / task_items.input_storage_object_id
-- reference takes, so no new reference can appear after the ref check and
-- before the row delete.
SELECT id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
FROM context69.library_storage_objects
WHERE id = $1
FOR UPDATE
