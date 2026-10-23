-- One deterministic cleanup row for the admin pagination/filter regression.
-- The caller controls every persisted timestamp so ordering and projection are
-- asserted against known values instead of the host clock.
INSERT INTO context69.task_docling_remote_cleanup (
    remote_job_id,
    task_id,
    item_id,
    remote_task_id,
    reason,
    status,
    outcome,
    attempt_count,
    next_attempt_at,
    created_at,
    updated_at,
    finished_at,
    cleanup_deadline_at
)
VALUES (
    $1,
    $2,
    $3,
    $4,
    'orphan',
    $5,
    $6,
    0,
    $7,
    $8,
    $9,
    $10,
    $11
);
