UPDATE context69.library_url_import_jobs
SET status = 'queued', updated_at = now()
WHERE status IN ('downloading', 'ingesting')
