SELECT
    id,
    document_id,
    author_user_id,
    producer,
    result_kind,
    schema_ref,
    source_record_hash,
    idempotency_key,
    result,
    provenance,
    created_at
FROM context69.document_results
WHERE document_id = $1
  AND ($2::text IS NULL OR producer = $2)
  AND ($3::text IS NULL OR result_kind = $3)
  AND ($4::bigint IS NULL OR id < $4)
ORDER BY id DESC
LIMIT $5
