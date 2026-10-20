INSERT INTO context69.library_files
    (id, group_id, filename, media_type, size_bytes, sha256, storage_rel_path,
     storage_object_id, ingest_status, visibility)
VALUES ($1, $2, $3, 'text/plain', $4, $5, $6, $7, 'succeeded', 'public')
