INSERT INTO context69.runtime_scheduler_settings (
    singleton,
    interval_secs,
    run_on_start,
    max_concurrency,
    job_id,
    valkey_url,
    interactive_max_concurrency,
    bulk_max_concurrency,
    maintenance_max_concurrency,
    ingest_max_concurrency,
    ingest_batches_per_segment,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now())
ON CONFLICT (singleton) DO UPDATE
SET interval_secs = EXCLUDED.interval_secs,
    run_on_start = EXCLUDED.run_on_start,
    max_concurrency = EXCLUDED.max_concurrency,
    job_id = EXCLUDED.job_id,
    valkey_url = EXCLUDED.valkey_url,
    interactive_max_concurrency = EXCLUDED.interactive_max_concurrency,
    bulk_max_concurrency = EXCLUDED.bulk_max_concurrency,
    maintenance_max_concurrency = EXCLUDED.maintenance_max_concurrency,
    ingest_max_concurrency = EXCLUDED.ingest_max_concurrency,
    ingest_batches_per_segment = EXCLUDED.ingest_batches_per_segment,
    updated_at = now()
