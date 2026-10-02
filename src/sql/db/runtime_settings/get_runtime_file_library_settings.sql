-- The S3 access key stays in the settings row because it is an identifier, not a
-- credential. The secret key is not projected: the shared encrypted store owns
-- it, and the service resolves it through that store alone.
SELECT
    storage_root,
    max_upload_size_mb,
    max_upload_request_size_mb,
    ingest_concurrency,
    url_import_concurrency,
    url_import_min_interval_ms,
    trusted_proxy_enabled,
    s3_endpoint,
    s3_region,
    s3_bucket,
    s3_prefix,
    s3_path_style,
    s3_access_key
FROM context69.runtime_file_library_settings
WHERE singleton = TRUE
