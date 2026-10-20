INSERT INTO context69.library_storage_objects
    (id, group_id, sha256, size_bytes, storage_backend, object_key)
VALUES ($1, $2, $3, $4, 'local', $5)
