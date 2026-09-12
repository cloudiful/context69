-- Durable physical-deletion outbox for released library source objects
-- (issue 332 phase 2, attempt 2).
--
-- Releasing a source detaches the file and records one cleanup intent instead
-- of deleting the storage-object row inside the release transaction. The
-- cleanup worker deletes the physical bytes first and only then the
-- storage-object row, so a crash or storage failure between the two leaves a
-- retryable intent rather than an unrecoverable orphan. The object identity
-- (id/key/sha/backend/group) is recorded here and re-validated under a row
-- lock before any deletion, so a newly referenced object is never deleted.
CREATE TABLE IF NOT EXISTS context69.library_storage_object_cleanup (
    id BIGSERIAL PRIMARY KEY,
    object_id UUID,
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    sha256 TEXT,
    object_key TEXT NOT NULL,
    storage_backend TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(object_key) <> ''),
    CHECK (storage_backend IN ('local', 's3')),
    CHECK (attempts >= 0)
);

-- At most one open intent per content-addressed object or legacy direct path.
CREATE UNIQUE INDEX IF NOT EXISTS uq_source_object_cleanup_open_object
    ON context69.library_storage_object_cleanup (object_id)
    WHERE object_id IS NOT NULL AND completed_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uq_source_object_cleanup_open_path
    ON context69.library_storage_object_cleanup (object_key)
    WHERE object_id IS NULL AND completed_at IS NULL;

-- Due-order scan for the retry worker; next_attempt_at pushes a repeatedly
-- failing intent behind fresh work so it cannot starve the queue.
CREATE INDEX IF NOT EXISTS idx_source_object_cleanup_due
    ON context69.library_storage_object_cleanup (next_attempt_at, id)
    WHERE completed_at IS NULL;
