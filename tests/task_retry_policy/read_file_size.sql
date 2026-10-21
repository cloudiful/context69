-- The size of the stored file.
SELECT size_bytes
FROM context69.library_files
WHERE id = $1
