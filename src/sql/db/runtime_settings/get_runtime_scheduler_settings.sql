SELECT interval_secs, run_on_start, max_concurrency, job_id, valkey_url
FROM context69.runtime_scheduler_settings
WHERE singleton = TRUE
