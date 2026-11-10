-- One page of staged translated chunks with their version, document and group.
SELECT
    s.chunk_id,
    s.document_id,
    s.chunk_index,
    s.chunk_text,
    s.record_hash,
    s.translation_id,
    t.target_locale,
    t.source_locale,
    t.provider_key,
    t.translated_title,
    t.translated_summary,
    d.group_id,
    g.group_key,
    g.full_path AS group_path,
    g.visibility,
    d.source_key,
    d.external_id,
    d.source_uri,
    d.published_at,
    d.updated_at_source,
    d.metadata_json
FROM context69.document_translation_rebuild_staging s
INNER JOIN context69.document_translation_versions t ON t.id = s.translation_id
INNER JOIN context69.documents d ON d.id = s.document_id
INNER JOIN context69.groups g ON g.id = d.group_id
WHERE s.target_collection = $1
  AND (
      $2::BIGINT IS NULL
      OR s.document_id > $2
      OR (s.document_id = $2 AND s.translation_id > $3)
      OR (s.document_id = $2 AND s.translation_id = $3 AND s.chunk_index > $4)
  )
ORDER BY s.document_id, s.translation_id, s.chunk_index
LIMIT $5
