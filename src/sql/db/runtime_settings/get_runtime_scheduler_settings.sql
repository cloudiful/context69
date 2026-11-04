SELECT interval_secs, run_on_start, max_concurrency, job_id, valkey_url,
       interactive_max_concurrency, bulk_max_concurrency,
       maintenance_max_concurrency, ingest_max_concurrency,
       ingest_batches_per_segment
FROM context69.runtime_scheduler_settings
WHERE singleton = TRUE
