WITH requested AS (
    SELECT source_key, external_id, ordinal
    FROM unnest($2::text[], $3::text[]) WITH ORDINALITY AS input(source_key, external_id, ordinal)
)
SELECT requested.ordinal AS "ordinal!", d.id AS document_id
FROM requested
LEFT JOIN context69.documents d
  ON d.group_id = $1
 AND d.source_key = requested.source_key
 AND d.external_id = requested.external_id
ORDER BY requested.ordinal
