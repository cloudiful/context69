-- The bounded occurrence links of a set of images, one extra row per image so
-- the caller can report `has_more` without a second count.
--
-- $1 group_id, $2 file_ids, $3 per-file limit.
SELECT file_id,
       document_id,
       group_id,
       role,
       ordinal,
       caption,
       context_text,
       source_uri,
       created_at,
       updated_at
FROM (
    SELECT links.*,
           row_number() OVER (
               PARTITION BY file_id
               ORDER BY role, ordinal, document_id
           ) AS occurrence_rank
    FROM context69.library_image_documents links
    WHERE links.group_id = $1
      AND links.file_id = ANY($2)
) ranked
WHERE occurrence_rank <= $3
ORDER BY file_id, role, ordinal, document_id
