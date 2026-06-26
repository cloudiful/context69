SELECT
    storage_root,
    max_upload_size_mb,
    max_upload_request_size_mb,
    ingest_concurrency,
    pdf_pages_per_task
FROM context69.runtime_file_library_settings
WHERE singleton = TRUE
