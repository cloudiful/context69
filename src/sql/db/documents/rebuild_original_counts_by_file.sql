-- New original live chunks of the staged target grouped by current file
-- association; documents without a unique file form the NULL-file group.
SELECT
    associated.file_id,
    count(*)::bigint AS "count!"
FROM (
    SELECT (
        SELECT f.file_id
        FROM context69.library_file_documents f
        WHERE f.document_id = s.document_id
        ORDER BY f.file_id
        LIMIT 1
    ) AS file_id
    FROM context69.document_chunk_rebuild_staging s
    WHERE s.target_collection = $1
) AS associated
GROUP BY associated.file_id
