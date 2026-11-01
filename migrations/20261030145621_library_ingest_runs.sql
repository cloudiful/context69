CREATE SEQUENCE context69.library_ingest_generation_seq AS BIGINT START WITH 1;

CREATE TABLE context69.library_ingest_runs (
    item_id UUID PRIMARY KEY REFERENCES context69.task_items(id) ON DELETE CASCADE,
    task_id UUID NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    file_id UUID NOT NULL REFERENCES context69.library_files(id) ON DELETE CASCADE,
    batch_id UUID NOT NULL UNIQUE,
    generation BIGINT NOT NULL UNIQUE,
    mode TEXT NOT NULL CHECK (mode IN ('first_insert', 'replacement')),
    source TEXT NOT NULL CHECK (source IN ('url', 'stored')),
    lease_token UUID NOT NULL,
    execution_task_id UUID NOT NULL,
    execution_generation BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ
);

CREATE INDEX idx_library_ingest_runs_file_state
    ON context69.library_ingest_runs (file_id, resolved_at);
