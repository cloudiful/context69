WITH hits AS (
    SELECT c.id AS chunk_id, d.id AS document_id, d.group_id, g.group_key,
           g.full_path AS group_path, d.visibility, d.source_key, d.external_id,
           d.title, d.summary, d.source_uri, d.published_at, c.chunk_index,
           c.chunk_text, d.metadata_json, 'original'::TEXT AS content_locale,
           tj.status AS translation_status
    FROM context69.document_chunks c
    JOIN context69.documents d ON d.id = c.document_id
    JOIN context69.groups g ON g.id = d.group_id
    LEFT JOIN LATERAL (
        SELECT status
        FROM context69.document_translation_jobs
        WHERE document_id = d.id AND target_locale = $4
        ORDER BY updated_at DESC
        LIMIT 1
    ) tj ON $4::TEXT IS NOT NULL
    WHERE c.id = ANY($1)
    UNION ALL
    SELECT c.id, d.id, d.group_id, g.group_key, g.full_path, d.visibility,
           d.source_key, d.external_id, v.translated_title, v.translated_summary,
           d.source_uri, d.published_at, c.chunk_index, c.chunk_text,
           d.metadata_json, v.target_locale, 'succeeded'::TEXT
    FROM context69.document_translation_chunks c
    JOIN context69.document_translation_versions v ON v.id = c.translation_id
    JOIN context69.documents d ON d.id = c.document_id AND d.record_hash = v.source_record_hash
    JOIN context69.groups g ON g.id = d.group_id
    WHERE c.id = ANY($1)
)
SELECT chunk_id AS "chunk_id!", document_id AS "document_id!",
       group_key AS "group_key!", group_path AS "group_path!",
       visibility AS "visibility!", source_key AS "source_key!",
       external_id AS "external_id!", title AS "title!", summary,
       source_uri AS "source_uri!", published_at, chunk_index AS "chunk_index!",
       chunk_text AS "chunk_text!", metadata_json AS "metadata_json!",
       content_locale AS "content_locale!", translation_status
FROM hits
WHERE (visibility = 'public' OR group_id = ANY($2))
  AND ($3::text IS NULL OR group_path = $3)
