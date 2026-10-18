INSERT INTO context69.library_files
    (id, group_id, visibility, folder_id, external_id, filename, media_type,
     size_bytes, sha256, storage_rel_path, ingest_status, error_message)
VALUES ($1, $2, 'public', NULL, $3, 'notes.txt', 'text/plain', $4, $5, $6, 'failed', 'awaiting first ingest')
