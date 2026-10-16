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
  AND author_user_id = $2
  AND producer = $3
  AND idempotency_key = $4
