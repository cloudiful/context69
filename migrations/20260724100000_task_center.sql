CREATE TABLE IF NOT EXISTS context69.tasks (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    group_id BIGINT REFERENCES context69.groups(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    group_path TEXT,
    source_key TEXT,
    total_count BIGINT NOT NULL DEFAULT 0,
    queued_count BIGINT NOT NULL DEFAULT 0,
    running_count BIGINT NOT NULL DEFAULT 0,
    succeeded_count BIGINT NOT NULL DEFAULT 0,
    failed_count BIGINT NOT NULL DEFAULT 0,
    cancelled_count BIGINT NOT NULL DEFAULT 0,
    failure_stage TEXT,
    error_summary TEXT,
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_tasks_user_created_at
    ON context69.tasks (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_tasks_status_lease
    ON context69.tasks (status, lease_until);

CREATE TABLE IF NOT EXISTS context69.task_items (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    resource_id TEXT,
    failure_stage TEXT,
    error_message TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    retryable BOOLEAN NOT NULL DEFAULT TRUE,
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (task_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_task_items_task_status
    ON context69.task_items (task_id, status, ordinal);

CREATE TABLE IF NOT EXISTS context69.task_attempts (
    id BIGSERIAL PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    item_id UUID REFERENCES context69.task_items(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL,
    status TEXT NOT NULL,
    retryable BOOLEAN NOT NULL DEFAULT TRUE,
    failure_stage TEXT,
    error_message TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS context69.task_idempotency_keys (
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    task_id UUID NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, idempotency_key)
);
