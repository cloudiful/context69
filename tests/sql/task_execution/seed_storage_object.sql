INSERT INTO context69.library_storage_objects (id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until) VALUES ($1, $2, $3, 3, 'local', $4, now() + interval '1 hour')
