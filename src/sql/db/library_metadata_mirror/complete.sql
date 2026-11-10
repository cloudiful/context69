DELETE FROM context69.library_file_metadata_mirror
WHERE file_id = $1
  AND version = $2
