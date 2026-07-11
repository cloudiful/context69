SELECT
    storage_root,
    max_upload_size_mb,
    max_upload_request_size_mb,
    ingest_concurrency,
    pdf_pages_per_task,
    s3_endpoint,
    s3_region,
    s3_bucket,
    s3_prefix,
    s3_path_style,
    s3_access_key,
    s3_secret_key
FROM context69.runtime_file_library_settings
WHERE singleton = TRUE
