SELECT id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
FROM context69.library_storage_objects
WHERE group_id = $1 AND sha256 = $2
