-- Make a staged object immediately reclaimable by the production sweep: both
-- its lease and its ordering key are older than the 24-hour grace. The literal
-- ordering keeps the fixture inside the bounded first page.
UPDATE context69.library_storage_objects
SET staging_lease_until = $2,
    updated_at = $2
WHERE id = $1
