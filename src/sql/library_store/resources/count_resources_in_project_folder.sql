SELECT COUNT(*)::BIGINT AS "count!"
FROM (
    SELECT name, NULL::TEXT AS media_type, NULL::TEXT AS ingest_status
    FROM context69.library_folders
    WHERE group_id = $1
      AND parent_id IS NOT DISTINCT FROM $2::UUID

    UNION ALL

    SELECT filename AS name, media_type, ingest_status
    FROM context69.library_files
    WHERE group_id = $1
      AND folder_id IS NOT DISTINCT FROM $2::UUID
) resources
WHERE (
       NULLIF(BTRIM($3::TEXT), '') IS NULL
    OR resources.name ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.media_type, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.ingest_status, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
)
AND ($4::TEXT IS NULL OR resources.ingest_status = $4::TEXT)
