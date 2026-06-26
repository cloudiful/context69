WITH query_terms AS (
    SELECT unnest($3::text[]) AS term
),
scored AS (
    SELECT
        c.id AS chunk_id,
        d.id AS document_id,
        g.group_key,
        p.project_key,
        d.visibility,
        d.source_key,
        d.external_id,
        d.title,
        d.summary,
        d.source_uri,
        d.published_at,
        c.chunk_index,
        c.chunk_text,
        d.metadata_json,
        lower(d.title) AS title_lc,
        lower(c.chunk_text) AS chunk_lc
    FROM context69.document_chunks c
    INNER JOIN context69.documents d ON d.id = c.document_id
    INNER JOIN context69.groups g ON g.id = d.group_id
    INNER JOIN context69.projects p ON p.id = d.project_id
    WHERE ($4::text IS NULL OR d.source_key = $4)
      AND ($5::text IS NULL OR g.group_key = $5)
      AND ($6::text IS NULL OR p.project_key = $6)
      AND ($7::date IS NULL OR d.published_at >= $7)
      AND ($8::date IS NULL OR d.published_at <= $8)
      AND (d.visibility = 'public' OR d.project_id = ANY($9))
      AND (
        lower(d.title) LIKE $2
        OR lower(c.chunk_text) LIKE $2
        OR (
            cardinality($3::text[]) > 0
            AND NOT EXISTS (
                SELECT 1
                FROM query_terms qt
                WHERE (lower(d.title) || ' ' || lower(c.chunk_text)) NOT LIKE ('%' || qt.term || '%')
            )
        )
      )
)
SELECT
    chunk_id,
    document_id,
    group_key,
    project_key,
    visibility,
    source_key,
    external_id,
    title,
    summary,
    source_uri,
    published_at,
    chunk_index,
    chunk_text,
    metadata_json,
    (
        CASE WHEN title_lc = $1 THEN 1.20 ELSE 0 END
        + CASE WHEN title_lc LIKE $2 THEN 1.00 ELSE 0 END
        + CASE WHEN chunk_lc LIKE $2 THEN 0.82 ELSE 0 END
        + CASE WHEN cardinality($3::text[]) > 0 THEN 0.35 ELSE 0 END
    )::real AS "keyword_score!",
    CASE
        WHEN title_lc = $1 THEN 'title_exact'
        WHEN title_lc LIKE $2 THEN 'title_phrase'
        WHEN chunk_lc LIKE $2 THEN 'chunk_phrase'
        ELSE 'all_terms'
    END AS "match_reason!"
FROM scored
ORDER BY "keyword_score!" DESC, published_at DESC NULLS LAST, document_id DESC, chunk_index ASC
LIMIT $10
