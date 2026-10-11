WITH inserted_group AS (
    INSERT INTO context69.groups (group_key, name, visibility, kind, full_path)
    VALUES ($1, 'Keyword candidate benchmark', 'public', 'shared', $2)
    RETURNING id
), inserted_documents AS (
    INSERT INTO context69.documents (
        group_id, source_key, external_id, title, summary, source_uri,
        published_at, updated_at_source, metadata_json, record_hash, visibility
    )
    SELECT g.id,
           'keyword-benchmark',
           'benchmark-' || sequence,
           CASE
               WHEN sequence = 1 OR sequence % 17 = 0 THEN 'twotermalpha'
               ELSE 'benchmark document ' || sequence
           END || CASE
               WHEN sequence = 1 OR sequence % 101 = 0 THEN ' threetermalpha'
               ELSE ''
           END,
           NULL,
           'https://example.test/keyword-benchmark/' || sequence,
           now() - sequence * interval '1 second',
           now(),
           '{"benchmark":true}'::jsonb,
           $3::text || ':' || sequence,
           'public'
    FROM inserted_group g
    CROSS JOIN generate_series(1, $4::integer) AS docs(sequence)
    RETURNING id, external_id
), inserted_chunks AS (
    INSERT INTO context69.document_chunks (id, document_id, chunk_index, chunk_text, record_hash)
    SELECT md5($3::text || ':' || d.external_id || ':' || chunk_index)::uuid,
           d.id,
           chunk_index,
           CASE WHEN d.external_id = 'benchmark-1' AND chunk_index = 0
                THEN 'rareneedle rarealpha rarebeta raregamma ' ELSE '' END
           || CASE WHEN d.external_id = 'benchmark-1' OR d.external_id LIKE 'benchmark-%'
                        AND substring(d.external_id from 11)::integer % 17 = 0
                   THEN 'twotermbeta ' ELSE '' END
           || CASE WHEN d.external_id = 'benchmark-1' OR d.external_id LIKE 'benchmark-%'
                        AND substring(d.external_id from 11)::integer % 101 = 0
                   THEN 'threetermbeta threetermgamma ' ELSE '' END
           || CASE WHEN substring(d.external_id from 11)::integer % 10 = 0
                   THEN 'commonneedle commonxneedle 中 ' ELSE '' END
           || repeat('fillersegment ', 64),
           $3::text || ':' || d.external_id
    FROM inserted_documents d
    CROSS JOIN generate_series(0, 7) AS chunks(chunk_index)
    RETURNING id
)
SELECT (SELECT count(*) FROM inserted_documents)::bigint AS document_count,
       (SELECT count(*) FROM inserted_chunks)::bigint AS chunk_count
