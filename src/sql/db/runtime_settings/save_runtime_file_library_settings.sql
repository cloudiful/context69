INSERT INTO context69.runtime_file_library_settings (
    singleton,
    storage_root,
    max_upload_size_mb,
    max_upload_request_size_mb,
    ingest_concurrency,
    pdf_pages_per_task,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, now())
ON CONFLICT (singleton) DO UPDATE
SET storage_root = EXCLUDED.storage_root,
    max_upload_size_mb = EXCLUDED.max_upload_size_mb,
    max_upload_request_size_mb = EXCLUDED.max_upload_request_size_mb,
    ingest_concurrency = EXCLUDED.ingest_concurrency,
    pdf_pages_per_task = EXCLUDED.pdf_pages_per_task,
    updated_at = now()
