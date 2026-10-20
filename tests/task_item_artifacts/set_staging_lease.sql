UPDATE context69.library_storage_objects
SET staging_lease_until = $2
WHERE id = $1
