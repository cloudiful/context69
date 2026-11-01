-- The source and status of one library file.
SELECT storage_object_id AS "storage_object_id",
       storage_rel_path AS "storage_rel_path",
       sha256 AS "sha256",
       size_bytes AS "size_bytes",
       ingest_status AS "ingest_status"
FROM context69.library_files
WHERE id = $1
