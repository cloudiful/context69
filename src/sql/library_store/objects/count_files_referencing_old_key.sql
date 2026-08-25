-- Live reference safety check for the legacy cleanup phase: an old key still
-- referenced by any library_files row must never be physically deleted.
SELECT count(*)::bigint AS "references!"
FROM context69.library_files
WHERE storage_rel_path = $1
