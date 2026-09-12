-- Legacy identity guard: a content-addressed storage-object row now owning the
-- same key means the path was re-promoted, so the legacy bytes must not be
-- deleted.
SELECT count(*)::bigint AS "count!"
FROM context69.library_storage_objects
WHERE object_key = $1
