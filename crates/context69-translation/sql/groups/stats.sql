SELECT
    COUNT(*) FILTER (WHERE j.status = 'queued')::BIGINT AS "queued_count!",
    COUNT(*) FILTER (WHERE j.status = 'running')::BIGINT AS "running_count!",
    COUNT(*) FILTER (WHERE j.status = 'succeeded')::BIGINT AS "succeeded_count!",
    COUNT(*) FILTER (WHERE j.status IN ('failed', 'quota_exceeded'))::BIGINT AS "failed_count!"
FROM context69.document_translation_jobs j
JOIN context69.documents d ON d.id = j.document_id
WHERE d.group_id = $1
