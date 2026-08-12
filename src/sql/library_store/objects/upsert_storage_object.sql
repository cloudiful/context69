INSERT INTO context69.library_storage_objects (
    id,
    group_id,
    sha256,
    size_bytes,
    storage_backend,
    object_key
)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (group_id, sha256) DO UPDATE
SET size_bytes = EXCLUDED.size_bytes,
    storage_backend = EXCLUDED.storage_backend,
    object_key = EXCLUDED.object_key,
    updated_at = now()
RETURNING id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
