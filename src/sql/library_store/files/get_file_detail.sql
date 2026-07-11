SELECT
    g.group_key AS "group_key!",
    g.full_path AS "group_path!",
    lf.visibility AS "visibility!",
    lf.id AS "file_id!",
    lf.folder_id,
    lf.filename AS "filename!",
    lf.media_type AS "media_type!",
    lf.size_bytes AS "size_bytes!",
    lf.sha256 AS "sha256!",
    lf.ingest_status AS "ingest_status!",
    lf.error_message,
    lf.created_at AS "created_at!",
    lf.updated_at AS "updated_at!",
    lf.ingested_at
FROM context69.library_files AS lf
INNER JOIN context69.groups AS g ON g.id = lf.group_id
WHERE lf.id = $1
