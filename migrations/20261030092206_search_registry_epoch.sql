-- Search registry epoch and registry entries (issue #1138 P3).
--
-- Coherence foundation for the future safe reingest design (#change-23319,
-- #change-23320): one logged epoch row is the reader/writer door, and one
-- registry entry row describes each in-flight run. No production writer
-- exists in this phase; searches only read the state, and the sole mutation
-- is the transactional bump primitive exercised by tests.
--
-- A consistent registry snapshot is one statement: the epoch and the entries
-- are read together (src/sql/db/search_registry/read_state.sql), so an empty
-- registry is the epoch row plus an empty entry list.

CREATE TABLE IF NOT EXISTS context69.search_registry_epoch (
    id SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    epoch BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO context69.search_registry_epoch (id, epoch)
VALUES (1, 0)
ON CONFLICT (id) DO NOTHING;

CREATE TABLE IF NOT EXISTS context69.search_registry_entries (
    run_id UUID PRIMARY KEY,
    file_id UUID,
    document_ids BIGINT[] NOT NULL DEFAULT '{}',
    staged_ids UUID[] NOT NULL DEFAULT '{}',
    retired_ids UUID[] NOT NULL DEFAULT '{}',
    baseline_record_hash TEXT,
    staged_record_hash TEXT,
    generation BIGINT NOT NULL,
    phase TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_search_registry_entries_phase
        CHECK (phase IN ('staging', 'switched', 'retired'))
);
