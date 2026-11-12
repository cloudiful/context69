-- Per-file original counts of one installed page only.
--
-- The aggregation is bounded by the page keyset, so a batch never scans the
-- whole staged snapshot. A document without a unique library file forms the
-- NULL-file group, exactly like the replaced per-file accounting.
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
      AND (
          $2::bigint IS NULL
          OR s.document_id > $2
          OR (s.document_id = $2 AND s.chunk_index > $3)
      )
      AND (
          s.document_id < $4
          OR (s.document_id = $4 AND s.chunk_index <= $5)
      )
) AS associated
GROUP BY associated.file_id
ORDER BY associated.file_id
