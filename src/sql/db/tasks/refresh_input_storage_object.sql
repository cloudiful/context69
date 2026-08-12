UPDATE context69.library_storage_objects
SET staging_lease_until = now() + interval '24 hours',
    updated_at = now()
WHERE id = $1
