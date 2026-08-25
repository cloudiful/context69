-- Record a physical-delete failure so the row is retried by a later run.
-- Never marks the row deleted; the next invocation reselects it.
UPDATE context69.library_legacy_object_cleanup
SET delete_error = $2
WHERE id = $1
  AND deleted_at IS NULL
