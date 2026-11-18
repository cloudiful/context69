-- Recheck the image file inside the locked link transaction.
--
-- $1 file_id, $2 group_id.
SELECT filename, media_type
FROM context69.library_files
WHERE id = $1 AND group_id = $2
