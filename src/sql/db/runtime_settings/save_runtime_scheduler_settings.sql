INSERT INTO context69.runtime_scheduler_settings (
    singleton,
    interval_secs,
    run_on_start,
    max_concurrency,
    job_id,
    valkey_url,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, now())
ON CONFLICT (singleton) DO UPDATE
SET interval_secs = EXCLUDED.interval_secs,
    run_on_start = EXCLUDED.run_on_start,
    max_concurrency = EXCLUDED.max_concurrency,
    job_id = EXCLUDED.job_id,
    valkey_url = EXCLUDED.valkey_url,
    updated_at = now()
