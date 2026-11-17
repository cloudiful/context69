-- Hydrate the parent headers of image occurrences inside the same group and
-- the caller's existing document read scope.
--
-- Only the document header fields are selected; no chunk text or raw content
-- crosses this query.
--
-- $1 group_id, $2 document_ids, $3 private_group_ids, $4 group_path.
SELECT d.id AS document_id,
       d.source_key,
       d.external_id,
       d.title,
       d.source_uri
FROM context69.documents d
INNER JOIN context69.groups g ON g.id = d.group_id
WHERE d.group_id = $1
  AND d.id = ANY($2)
  AND (g.visibility = 'public' OR d.group_id = ANY($3))
  AND ($4::text IS NULL OR g.full_path = $4)
