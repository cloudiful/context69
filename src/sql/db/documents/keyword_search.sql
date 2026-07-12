WITH query_terms AS (
    SELECT unnest($3::text[]) AS term
), searchable AS (
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
        WHERE document_id = d.id AND target_locale = $10
        ORDER BY updated_at DESC
        LIMIT 1
    ) tj ON $10::TEXT IS NOT NULL
    UNION ALL
    SELECT c.id, d.id, d.group_id, g.group_key, g.full_path, d.visibility,
           d.source_key, d.external_id, v.translated_title, v.translated_summary,
           d.source_uri, d.published_at, c.chunk_index, c.chunk_text,
           d.metadata_json, v.target_locale, 'succeeded'::TEXT
    FROM context69.document_translation_chunks c
    JOIN context69.document_translation_versions v ON v.id = c.translation_id
    JOIN context69.documents d ON d.id = c.document_id AND d.record_hash = v.source_record_hash
    JOIN context69.groups g ON g.id = d.group_id
    WHERE $10::TEXT IS NOT NULL AND v.target_locale = $10
), scored AS (
    SELECT *, lower(title) AS title_lc, lower(chunk_text) AS chunk_lc
    FROM searchable
    WHERE ($4::text IS NULL OR source_key = $4)
      AND ($5::text IS NULL OR group_path = $5)
      AND ($6::timestamptz IS NULL OR published_at >= $6)
      AND ($7::timestamptz IS NULL OR published_at <= $7)
      AND (visibility = 'public' OR group_id = ANY($8))
      AND (content_locale = 'original' OR content_locale = $10)
      AND (
        lower(title) LIKE $2 OR lower(chunk_text) LIKE $2
        OR (cardinality($3::text[]) > 0 AND NOT EXISTS (
            SELECT 1 FROM query_terms qt
            WHERE (lower(title) || ' ' || lower(chunk_text)) NOT LIKE ('%' || qt.term || '%')
        ))
      )
)
SELECT chunk_id AS "chunk_id!", document_id AS "document_id!",
       group_key AS "group_key!", group_path AS "group_path!",
       visibility AS "visibility!", source_key AS "source_key!",
       external_id AS "external_id!", title AS "title!", summary,
       source_uri AS "source_uri!", published_at, chunk_index AS "chunk_index!",
       chunk_text AS "chunk_text!", metadata_json AS "metadata_json!",
       content_locale AS "content_locale!", translation_status,
       (CASE WHEN title_lc = $1 THEN 1.20 ELSE 0 END
        + CASE WHEN title_lc LIKE $2 THEN 1.00 ELSE 0 END
        + CASE WHEN chunk_lc LIKE $2 THEN 0.82 ELSE 0 END
        + CASE WHEN cardinality($3::text[]) > 0 THEN 0.35 ELSE 0 END)::real AS "keyword_score!",
       CASE WHEN title_lc = $1 THEN 'title_exact'
            WHEN title_lc LIKE $2 THEN 'title_phrase'
            WHEN chunk_lc LIKE $2 THEN 'chunk_phrase'
            ELSE 'all_terms' END AS "match_reason!"
FROM scored
ORDER BY "keyword_score!" DESC, published_at DESC NULLS LAST, document_id DESC, chunk_index ASC
LIMIT $9
