-- Record one open physical-deletion intent. The partial unique indexes make a
-- second release of the same object or legacy path a no-op while an intent is
-- still open.
INSERT INTO context69.library_storage_object_cleanup
    (object_id, group_id, sha256, object_key, storage_backend)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT DO NOTHING
RETURNING id
