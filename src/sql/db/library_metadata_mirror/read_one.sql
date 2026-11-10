SELECT file_id, version, attempts
FROM context69.library_file_metadata_mirror
WHERE file_id = $1
