SELECT id
FROM context69.library_url_import_jobs
WHERE status IN ('queued', 'downloading', 'ingesting')
ORDER BY created_at, id
