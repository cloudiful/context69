WITH RECURSIVE subtree(id) AS (
    SELECT folder.id
    FROM context69.library_folders folder
    WHERE $5::BOOL IS TRUE
      AND $2::UUID IS NOT NULL
      AND folder.id = $2::UUID
      AND ($1::BIGINT IS NULL OR folder.group_id = $1)
    UNION ALL
    SELECT child.id
    FROM context69.library_folders child
    JOIN subtree ON child.parent_id = subtree.id
    WHERE ($1::BIGINT IS NULL OR child.group_id = $1)
)
SELECT COUNT(*)::BIGINT AS "count!"
FROM (
    SELECT folder.id, folder.parent_id, folder.name, NULL::TEXT AS media_type, NULL::TEXT AS ingest_status
    FROM context69.library_folders folder
    WHERE ($1::BIGINT IS NULL OR folder.group_id = $1)
      AND (
        ($5::BOOL IS FALSE AND folder.parent_id IS NOT DISTINCT FROM $2::UUID)
        OR ($5::BOOL IS TRUE AND $2::UUID IS NULL)
        OR ($5::BOOL IS TRUE AND $2::UUID IS NOT NULL AND folder.id IN (SELECT id FROM subtree WHERE id IS DISTINCT FROM $2::UUID))
      )

    UNION ALL

    SELECT file.id, file.folder_id AS parent_id, file.filename AS name, file.media_type, file.ingest_status
    FROM context69.library_files file
    WHERE ($1::BIGINT IS NULL OR file.group_id = $1)
      AND (
        ($5::BOOL IS FALSE AND file.folder_id IS NOT DISTINCT FROM $2::UUID)
        OR ($5::BOOL IS TRUE AND $2::UUID IS NULL)
        OR ($5::BOOL IS TRUE AND $2::UUID IS NOT NULL AND file.folder_id IN (SELECT id FROM subtree))
      )
) resources
WHERE (
       NULLIF(BTRIM($3::TEXT), '') IS NULL
    OR resources.name ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.media_type, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
    OR COALESCE(resources.ingest_status, '') ILIKE '%' || BTRIM($3::TEXT) || '%'
)
AND ($4::TEXT IS NULL OR resources.ingest_status = $4::TEXT)
