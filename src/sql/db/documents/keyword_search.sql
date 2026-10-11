-- Candidate rows are narrowed by independent, indexable title and chunk predicates;
-- the final filter below remains the semantic guard before global ordering/limit.
WITH query_terms AS (
    SELECT unnest($3::text[]) AS term
), distinct_terms AS (
    SELECT DISTINCT term
    FROM query_terms
    WHERE term <> ''
), metadata_filter_input AS (
    SELECT
        filter->>'path' AS path,
        filter->>'operator' AS operator,
        filter->'value' AS value,
        filter->'min' AS min_value,
        filter->'max' AS max_value
    FROM jsonb_array_elements($11::jsonb) AS filter
), eligible_original_documents AS (
    SELECT d.id AS document_id
    FROM context69.documents d
    JOIN context69.groups g ON g.id = d.group_id
    WHERE ($4::text IS NULL OR d.source_key = $4)
      AND ($5::text IS NULL OR g.full_path = $5)
      AND ($6::timestamptz IS NULL OR d.published_at >= $6)
      AND ($7::timestamptz IS NULL OR d.published_at <= $7)
      AND (g.visibility = 'public' OR d.group_id = ANY($8))
), eligible_translation_versions AS (
    SELECT v.id AS translation_id, d.id AS document_id, v.target_locale
    FROM context69.document_translation_versions v
    JOIN context69.documents d
      ON d.id = v.document_id AND d.record_hash = v.source_record_hash
    JOIN context69.groups g ON g.id = d.group_id
    WHERE $10::text IS NOT NULL
      AND v.target_locale = $10
      AND ($4::text IS NULL OR d.source_key = $4)
      AND ($5::text IS NULL OR g.full_path = $5)
      AND ($6::timestamptz IS NULL OR d.published_at >= $6)
      AND ($7::timestamptz IS NULL OR d.published_at <= $7)
      AND (g.visibility = 'public' OR d.group_id = ANY($8))
), original_title_phrase_documents AS (
    SELECT d.id AS document_id
    FROM context69.documents d
    JOIN eligible_original_documents eligible ON eligible.document_id = d.id
    WHERE lower(d.title) LIKE $2
), translated_title_phrase_versions AS (
    SELECT eligible.translation_id, eligible.document_id, eligible.target_locale
    FROM eligible_translation_versions eligible
    JOIN context69.document_translation_versions v ON v.id = eligible.translation_id
    WHERE lower(v.translated_title) LIKE $2
), original_phrase_rows AS (
    SELECT 'original'::text AS origin, c.id AS chunk_id,
           c.document_id, 'original'::text AS content_locale
    FROM original_title_phrase_documents title_match
    JOIN context69.document_chunks c ON c.document_id = title_match.document_id
    UNION
    SELECT 'original'::text, c.id, c.document_id, 'original'::text
    FROM context69.document_chunks c
    JOIN eligible_original_documents eligible ON eligible.document_id = c.document_id
    WHERE lower(c.chunk_text) LIKE $2
), translated_phrase_rows AS (
    SELECT 'translation'::text AS origin, c.id AS chunk_id,
           c.document_id, title_match.target_locale AS content_locale
    FROM translated_title_phrase_versions title_match
    JOIN context69.document_translation_chunks c
      ON c.translation_id = title_match.translation_id
     AND c.document_id = title_match.document_id
     AND c.target_locale = title_match.target_locale
    UNION
    SELECT 'translation'::text, c.id, c.document_id, eligible.target_locale
    FROM context69.document_translation_chunks c
    JOIN eligible_translation_versions eligible
      ON eligible.translation_id = c.translation_id
     AND eligible.document_id = c.document_id
     AND eligible.target_locale = c.target_locale
    WHERE lower(c.chunk_text) LIKE $2
), phrase_rows AS (
    SELECT origin, chunk_id, document_id, content_locale
    FROM original_phrase_rows
    UNION
    SELECT origin, chunk_id, document_id, content_locale
    FROM translated_phrase_rows
), original_title_term_rows AS (
    SELECT term.term, 'original'::text AS origin, c.id AS chunk_id,
           c.document_id, 'original'::text AS content_locale
    FROM distinct_terms term
    JOIN context69.documents d
      ON lower(d.title) LIKE ('%' || term.term || '%')
    JOIN eligible_original_documents eligible ON eligible.document_id = d.id
    JOIN context69.document_chunks c ON c.document_id = d.id
), original_chunk_term_rows AS (
    SELECT term.term, 'original'::text AS origin, c.id AS chunk_id,
           c.document_id, 'original'::text AS content_locale
    FROM distinct_terms term
    JOIN context69.document_chunks c
      ON lower(c.chunk_text) LIKE ('%' || term.term || '%')
    JOIN eligible_original_documents eligible ON eligible.document_id = c.document_id
), translated_title_term_rows AS (
    SELECT term.term, 'translation'::text AS origin, c.id AS chunk_id,
           c.document_id, eligible.target_locale AS content_locale
    FROM distinct_terms term
    JOIN eligible_translation_versions eligible ON TRUE
    JOIN context69.document_translation_versions v
      ON v.id = eligible.translation_id
    JOIN context69.document_translation_chunks c
      ON c.translation_id = eligible.translation_id
     AND c.document_id = eligible.document_id
     AND c.target_locale = eligible.target_locale
    WHERE lower(v.translated_title) LIKE ('%' || term.term || '%')
), translated_chunk_term_rows AS (
    SELECT term.term, 'translation'::text AS origin, c.id AS chunk_id,
           c.document_id, eligible.target_locale AS content_locale
    FROM distinct_terms term
    JOIN context69.document_translation_chunks c
      ON lower(c.chunk_text) LIKE ('%' || term.term || '%')
    JOIN eligible_translation_versions eligible
      ON eligible.translation_id = c.translation_id
     AND eligible.document_id = c.document_id
     AND eligible.target_locale = c.target_locale
), term_rows AS (
    SELECT term, origin, chunk_id, document_id, content_locale
    FROM original_title_term_rows
    UNION ALL
    SELECT term, origin, chunk_id, document_id, content_locale
    FROM original_chunk_term_rows
    UNION ALL
    SELECT term, origin, chunk_id, document_id, content_locale
    FROM translated_title_term_rows
    UNION ALL
    SELECT term, origin, chunk_id, document_id, content_locale
    FROM translated_chunk_term_rows
), all_term_rows AS (
    SELECT origin, chunk_id, document_id, content_locale
    FROM term_rows
    GROUP BY origin, chunk_id, document_id, content_locale
    HAVING COUNT(DISTINCT term) = (SELECT COUNT(*) FROM distinct_terms)
), candidate_rows AS (
    SELECT origin, chunk_id, document_id, content_locale
    FROM phrase_rows
    UNION
    SELECT origin, chunk_id, document_id, content_locale
    FROM all_term_rows
    WHERE cardinality($3::text[]) > 0
), searchable AS (
    SELECT c.id AS chunk_id, d.id AS document_id, d.group_id, g.group_key,
           g.full_path AS group_path, g.visibility, d.source_key, d.external_id,
           d.title, d.summary, d.source_uri, d.published_at, c.chunk_index,
           c.chunk_text, d.metadata_json, 'original'::text AS content_locale,
           tj.status AS translation_status
    FROM candidate_rows candidate
    JOIN context69.document_chunks c
      ON c.id = candidate.chunk_id AND c.document_id = candidate.document_id
    JOIN context69.documents d ON d.id = c.document_id
    JOIN context69.groups g ON g.id = d.group_id
    LEFT JOIN LATERAL (
        SELECT status
        FROM context69.document_translation_jobs
        WHERE document_id = d.id AND target_locale = $10
        ORDER BY updated_at DESC
        LIMIT 1
    ) tj ON $10::text IS NOT NULL
    WHERE candidate.origin = 'original'
      AND candidate.content_locale = 'original'
    UNION ALL
    SELECT c.id, d.id, d.group_id, g.group_key, g.full_path, g.visibility,
           d.source_key, d.external_id, v.translated_title, v.translated_summary,
           d.source_uri, d.published_at, c.chunk_index, c.chunk_text,
           d.metadata_json, v.target_locale, 'succeeded'::text
    FROM candidate_rows candidate
    JOIN context69.document_translation_chunks c
      ON c.id = candidate.chunk_id AND c.document_id = candidate.document_id
     AND c.target_locale = candidate.content_locale
    JOIN context69.document_translation_versions v
      ON v.id = c.translation_id AND v.target_locale = candidate.content_locale
    JOIN context69.documents d
      ON d.id = c.document_id AND d.record_hash = v.source_record_hash
    JOIN context69.groups g ON g.id = d.group_id
    WHERE candidate.origin = 'translation'
      AND v.target_locale = $10
), scored AS (
    SELECT *, lower(title) AS title_lc, lower(chunk_text) AS chunk_lc
    FROM searchable
    WHERE ($4::text IS NULL OR source_key = $4)
      AND ($5::text IS NULL OR group_path = $5)
      AND ($6::timestamptz IS NULL OR published_at >= $6)
      AND ($7::timestamptz IS NULL OR published_at <= $7)
      AND (visibility = 'public' OR group_id = ANY($8))
      AND (content_locale = 'original' OR content_locale = $10)
      AND NOT EXISTS (
          SELECT 1
          FROM metadata_filter_input filter
          CROSS JOIN LATERAL (
              SELECT metadata_json #> string_to_array(filter.path, '.') AS found
          ) value
          WHERE NOT CASE filter.operator
              WHEN 'exists' THEN
                  (value.found IS NOT NULL AND value.found <> 'null'::jsonb)
                  = CASE
                        WHEN jsonb_typeof(filter.value) = 'boolean'
                        THEN (filter.value #>> '{}')::boolean
                        ELSE TRUE
                    END
              WHEN 'eq' THEN value.found = filter.value
              WHEN 'in' THEN filter.value @> jsonb_build_array(value.found)
              WHEN 'contains' THEN value.found @> jsonb_build_array(filter.value)
              WHEN 'range' THEN
                  CASE
                      WHEN jsonb_typeof(value.found) = 'number'
                           AND (filter.min_value IS NULL OR jsonb_typeof(filter.min_value) = 'number')
                           AND (filter.max_value IS NULL OR jsonb_typeof(filter.max_value) = 'number') THEN
                          (filter.min_value IS NULL OR value.found::numeric >= filter.min_value::numeric)
                          AND (filter.max_value IS NULL OR value.found::numeric <= filter.max_value::numeric)
                      WHEN jsonb_typeof(value.found) = 'string' THEN
                          (filter.min_value IS NULL OR value.found #>> '{}' >= filter.min_value #>> '{}')
                          AND (filter.max_value IS NULL OR value.found #>> '{}' <= filter.max_value #>> '{}')
                      ELSE FALSE
                  END
              ELSE FALSE
          END
      )
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
