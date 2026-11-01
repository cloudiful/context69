-- One caller-produced result of a document, as an earlier delivery left it.
INSERT INTO context69.document_results (
    document_id, author_user_id, producer, result_kind, schema_ref,
    source_record_hash, idempotency_key, result
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
