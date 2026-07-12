INSERT INTO context69.document_translation_jobs (
    id, document_id, target_locale, requested_source_locale, source_record_hash
)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (document_id, target_locale, source_record_hash)
    WHERE status IN ('queued', 'running')
DO UPDATE SET updated_at = context69.document_translation_jobs.updated_at
RETURNING *
