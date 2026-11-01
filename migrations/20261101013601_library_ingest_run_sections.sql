-- Durable final document identity of the new sections of one library ingest
-- run (issue 1138 P30).
--
-- A replacement or atomic first insert reserves the final document id of every
-- section it did not retain, so the id is already decided before the first
-- external point write. Recording that decision keyed by the durable run and
-- the section key makes it survive a transient embedding, upsert or
-- verification failure: a retry of the same run reuses the exact id instead of
-- reserving a new one, while a terminal retry that claims a fresh generation
-- starts without a mapping and allocates its own ids.
--
-- Only the reserved identity lives here: no placeholder document or version
-- row is created, and the mapping is not part of any visible catalog. It
-- belongs to its run and is removed with it.
CREATE TABLE context69.library_ingest_run_sections (
    run_id UUID NOT NULL REFERENCES context69.library_ingest_runs(batch_id) ON DELETE CASCADE,
    section_key TEXT NOT NULL,
    file_id UUID NOT NULL,
    section_external_id TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, section_key)
);

CREATE INDEX idx_library_ingest_run_sections_document
    ON context69.library_ingest_run_sections (document_id);
