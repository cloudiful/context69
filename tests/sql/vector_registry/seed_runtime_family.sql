-- The runtime settings family a source-rerun deployment needs (issue 1212 R4).
--
-- The desired vector identity covers the qdrant and embedding rows only, while
-- the URL download path reads the whole runtime settings family. These rows
-- carry the normal defaults a fresh deployment writes, so the fixture never
-- removes a row to make production admission pass.
INSERT INTO context69.runtime_scheduler_settings
    (singleton, interval_secs, run_on_start, max_concurrency, job_id)
VALUES (TRUE, 3600, TRUE, 2, 'source-rerun')
ON CONFLICT (singleton) DO NOTHING;

INSERT INTO context69.runtime_chunking_settings
    (singleton, max_chars, overlap_chars, unit, tokenizer, max_tokens, overlap_tokens)
VALUES (TRUE, 1000, 100, 'chars', 'cl100k_base', 1024, 128)
ON CONFLICT (singleton) DO NOTHING;

INSERT INTO context69.runtime_file_library_settings
    (singleton, storage_root, max_upload_size_mb, max_upload_request_size_mb,
     ingest_concurrency, url_import_concurrency, url_import_min_interval_ms,
     trusted_proxy_enabled, s3_path_style,
     registry_max_ids_total, registry_max_ids_per_file, registry_max_ids_per_run)
VALUES (TRUE, '/tmp/context69-source-rerun', 10, 10, 1, 1, 1000, FALSE, FALSE,
        32768, 16384, 8192)
ON CONFLICT (singleton) DO NOTHING;
