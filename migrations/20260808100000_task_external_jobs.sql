CREATE TABLE IF NOT EXISTS context69.task_external_jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id uuid NOT NULL REFERENCES context69.task_items(id) ON DELETE CASCADE,
    provider text NOT NULL,
    remote_task_id text NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    remote_status text,
    submitted_at timestamptz NOT NULL DEFAULT now(),
    last_polled_at timestamptz,
    next_poll_at timestamptz NOT NULL DEFAULT now(),
    deadline_at timestamptz,
    error_message text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT chk_task_external_jobs_status CHECK (status IN ('pending', 'running', 'success', 'failure', 'timed_out', 'cancelled'))
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_task_external_jobs_item_provider
    ON context69.task_external_jobs (item_id, provider);

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_next_poll
    ON context69.task_external_jobs (status, next_poll_at);
