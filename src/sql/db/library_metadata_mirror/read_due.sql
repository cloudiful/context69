SELECT file_id, version, attempts
FROM context69.library_file_metadata_mirror
WHERE next_attempt_at <= now()
ORDER BY next_attempt_at, file_id
LIMIT $1
