WITH jobs AS (
    SELECT status, created_at, updated_at, requires_docling
    FROM context69.library_ingest_jobs

    UNION ALL

    SELECT status, created_at, updated_at, FALSE AS requires_docling
    FROM context69.library_url_import_jobs
)
SELECT
    COUNT(*) FILTER (WHERE status = 'pending') AS "pending_count!: i64",
    COUNT(*) FILTER (WHERE status = 'queued') AS "queued_count!: i64",
    MIN(created_at) FILTER (WHERE status = 'pending') AS oldest_pending_at,
    MIN(created_at) FILTER (WHERE status = 'queued') AS oldest_queued_at,
    COUNT(*) FILTER (
        WHERE status = 'failed'
          AND updated_at >= now() - INTERVAL '15 minutes'
    ) AS "recent_failure_count!: i64",
    COUNT(*) FILTER (
        WHERE requires_docling
          AND status IN ('pending', 'running')
    ) AS "docling_required_count!: i64"
FROM jobs
