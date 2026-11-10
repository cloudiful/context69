UPDATE context69.library_file_metadata_mirror
SET attempts = attempts + 1,
    next_attempt_at = now() + make_interval(secs => ($3)::int)
WHERE file_id = $1
  AND version = $2
