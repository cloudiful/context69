INSERT INTO context69.document_extraction_jobs (
    id, document_id, template_key, template_version, source_record_hash, parameters
)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (document_id, template_key, source_record_hash)
    WHERE status IN ('queued', 'running')
DO UPDATE SET updated_at = context69.document_extraction_jobs.updated_at
RETURNING *

