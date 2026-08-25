-- Conditional success marker for the legacy cleanup phase: only lands while
-- the row is still open, so a concurrent completion or retry can never
-- double-write the timestamp. Runs after the physical delete succeeded (or
-- the object was already missing), never before.
UPDATE context69.library_legacy_object_cleanup
SET deleted_at = now(),
    delete_error = NULL
WHERE id = $1
  AND deleted_at IS NULL
RETURNING id
