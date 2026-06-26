UPDATE context69.sync_runs
SET status = $2,
    records_seen = $3,
    records_changed = $4,
    chunks_upserted = $5,
    error_message = $6,
    finished_at = now(),
    updated_at = now()
WHERE id = $1
