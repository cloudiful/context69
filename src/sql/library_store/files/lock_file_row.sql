-- lock_file_row.sql (issue 1138, P19)
--
-- Read and lock one library file row as the first statement of the metadata
-- publication transaction. The replacement switch takes the same row first, so
-- both writers serialize on it and the per-document CAS below then refuses a
-- baseline that a switch already moved.
SELECT
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    folder_id,
    external_id,
    source_uri,
    published_at,
    metadata_json,
    filename,
    media_type,
    size_bytes,
    sha256,
    storage_rel_path,
    ingest_status,
    error_message,
    created_at,
    updated_at,
    ingested_at
FROM context69.library_files
WHERE id = $1
FOR UPDATE
