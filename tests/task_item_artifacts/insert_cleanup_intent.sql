INSERT INTO context69.library_storage_object_cleanup
    (object_id, group_id, sha256, object_key, storage_backend)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT DO NOTHING
