SELECT
    r.id,
    r.document_id,
    r.author_user_id,
    r.producer,
    r.result_kind,
    r.schema_ref,
    r.source_record_hash,
    r.idempotency_key,
    r.result,
    r.provenance,
    r.created_at
FROM context69.document_results r
WHERE r.document_id = $1
  AND ($2::text IS NULL OR r.producer = $2)
  AND ($3::text IS NULL OR r.result_kind = $3)
  AND ($4::bigint IS NULL OR r.id < $4)
  AND (NOT $5::boolean OR r.source_record_hash = $6::text)
  AND (
    NOT $7::boolean
    OR r.id = (
      SELECT max(latest.id)
      FROM context69.document_results latest
      WHERE latest.document_id = r.document_id
        AND latest.producer = r.producer
        AND latest.result_kind = r.result_kind
        AND latest.source_record_hash = $6::text
    )
  )
ORDER BY r.id DESC
LIMIT $8
