-- One page of staged original chunks with their document and group fields.
SELECT
    s.chunk_id,
    s.document_id,
    s.chunk_index,
    s.chunk_text,
    s.record_hash,
    d.group_id,
    g.group_key,
    g.full_path AS group_path,
    g.visibility,
    d.source_key,
    d.external_id,
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    d.updated_at_source,
    d.metadata_json
FROM context69.document_chunk_rebuild_staging s
INNER JOIN context69.documents d ON d.id = s.document_id
INNER JOIN context69.groups g ON g.id = d.group_id
WHERE s.target_collection = $1
  AND (
      $2::BIGINT IS NULL
      OR s.document_id > $2
      OR (s.document_id = $2 AND s.chunk_index > $3)
  )
ORDER BY s.document_id, s.chunk_index
LIMIT $4
