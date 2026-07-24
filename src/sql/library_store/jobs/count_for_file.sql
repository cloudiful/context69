SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.library_ingest_jobs
WHERE file_id = $1
