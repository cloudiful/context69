INSERT INTO context69.library_file_metadata_mirror (file_id, version)
VALUES ($1, 1)
ON CONFLICT (file_id) DO UPDATE
SET version = context69.library_file_metadata_mirror.version + 1,
    attempts = 0,
    next_attempt_at = now()
