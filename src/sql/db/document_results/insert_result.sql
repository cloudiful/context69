INSERT INTO context69.document_results (
    document_id,
    author_user_id,
    producer,
    result_kind,
    schema_ref,
    source_record_hash,
    idempotency_key,
    result,
    provenance
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
RETURNING
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
