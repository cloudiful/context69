WITH resources AS (
    SELECT
        'folder'::TEXT AS resource_kind,
        folder.id,
        groups.group_key,
        groups.full_path AS group_path,
        folder.visibility,
        folder.parent_id AS parent_folder_id,
        folder.name,
        NULL::TEXT AS media_type,
        NULL::BIGINT AS size_bytes,
        NULL::TEXT AS ingest_status,
        NULL::TEXT AS error_message,
        (SELECT COUNT(*) FROM context69.library_folders child WHERE child.parent_id = folder.id)::BIGINT AS child_folder_count,
        (SELECT COUNT(*) FROM context69.library_files file WHERE file.folder_id = folder.id)::BIGINT AS file_count,
        (SELECT COUNT(*) FROM context69.library_files file WHERE file.folder_id = folder.id AND file.ingest_status IN ('pending', 'running'))::BIGINT AS processing_count,
        EXISTS (SELECT 1 FROM context69.library_files file WHERE file.folder_id = folder.id AND LOWER(file.filename) = 'source.json') AS is_source_folder,
        (folder.name = 'records' AND folder.parent_id IS NOT NULL) AS is_source_records_folder,
        folder.created_at,
        folder.updated_at
    FROM context69.library_folders folder
    JOIN context69.groups groups ON groups.id = folder.group_id
    WHERE ($1::BIGINT IS NULL OR folder.group_id = $1)
      AND folder.parent_id IS NOT DISTINCT FROM $2::UUID

    UNION ALL

    SELECT
        'file'::TEXT AS resource_kind,
        file.id,
        groups.group_key,
        groups.full_path AS group_path,
        file.visibility,
        file.folder_id AS parent_folder_id,
        file.filename AS name,
        file.media_type,
        file.size_bytes,
        file.ingest_status,
        file.error_message,
        0::BIGINT AS child_folder_count,
        0::BIGINT AS file_count,
        0::BIGINT AS processing_count,
        FALSE AS is_source_folder,
        FALSE AS is_source_records_folder,
        file.created_at,
        file.updated_at
    FROM context69.library_files file
    JOIN context69.groups groups ON groups.id = file.group_id
    WHERE ($1::BIGINT IS NULL OR file.group_id = $1)
      AND file.folder_id IS NOT DISTINCT FROM $2::UUID
)
SELECT
    resource_kind AS "resource_kind!",
    id AS "id!",
    group_key AS "group_key!",
    group_path AS "group_path!",
    visibility AS "visibility!",
    parent_folder_id,
    name AS "name!",
    media_type,
    size_bytes,
    ingest_status,
    error_message,
    child_folder_count AS "child_folder_count!",
    file_count AS "file_count!",
    processing_count AS "processing_count!",
    is_source_folder AS "is_source_folder!",
    is_source_records_folder AS "is_source_records_folder!",
    created_at AS "created_at!",
    updated_at AS "updated_at!"
FROM resources
WHERE (
       NULLIF(BTRIM($3::TEXT), '') IS NULL
    OR resources.name ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.media_type, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.ingest_status, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
)
AND ($4::TEXT IS NULL OR resources.ingest_status = $4::TEXT)
ORDER BY
    CASE WHEN $5 = 'name' AND $6 = 'asc' THEN LOWER(name) END ASC NULLS LAST,
    CASE WHEN $5 = 'name' AND $6 = 'desc' THEN LOWER(name) END DESC NULLS LAST,
    CASE WHEN $5 = 'type' AND $6 = 'asc' THEN resource_kind || ':' || COALESCE(media_type, '') END ASC NULLS LAST,
    CASE WHEN $5 = 'type' AND $6 = 'desc' THEN resource_kind || ':' || COALESCE(media_type, '') END DESC NULLS LAST,
    CASE WHEN $5 = 'status' AND $6 = 'asc' THEN ingest_status END ASC NULLS LAST,
    CASE WHEN $5 = 'status' AND $6 = 'desc' THEN ingest_status END DESC NULLS LAST,
    CASE WHEN $5 = 'size' AND $6 = 'asc' THEN size_bytes END ASC NULLS LAST,
    CASE WHEN $5 = 'size' AND $6 = 'desc' THEN size_bytes END DESC NULLS LAST,
    CASE WHEN $5 = 'updated_at' AND $6 = 'asc' THEN updated_at END ASC NULLS LAST,
    CASE WHEN $5 = 'updated_at' AND $6 = 'desc' THEN updated_at END DESC NULLS LAST,
    LOWER(name) ASC,
    id ASC
LIMIT $7 OFFSET $8
