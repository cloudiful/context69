-- Durable record of legacy UUID direct-path objects that were migrated into
-- the content-addressed layout. Rows intentionally have no foreign keys so the
-- cleanup data survives deletion of the source library_files row (or even the
-- owning group) until the separate old-key cleanup phase deletes the physical
-- object.
CREATE TABLE IF NOT EXISTS context69.library_legacy_object_cleanup (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    group_id BIGINT NOT NULL,
    file_id UUID NOT NULL,
    old_key TEXT NOT NULL,
    migrated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    cleanup_eligible_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    delete_error TEXT,
    UNIQUE (file_id, old_key),
    CHECK (btrim(old_key) <> ''),
    CHECK (deleted_at IS NULL OR deleted_at >= migrated_at)
);

CREATE INDEX IF NOT EXISTS idx_library_legacy_object_cleanup_eligible
    ON context69.library_legacy_object_cleanup (cleanup_eligible_at)
    WHERE deleted_at IS NULL;
