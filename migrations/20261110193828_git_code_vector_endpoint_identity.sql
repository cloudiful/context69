-- Destination-endpoint identity and scheduler deferral for Git cleanup
-- (issue #28 phase P13).
--
-- `git_generation_code_vectors` now records a hash of the normalized Qdrant
-- endpoint that owns a generation's points. Cleanup may remove points only when
-- the current runtime endpoint has the same identity; a different or unknown
-- endpoint keeps the row so ownership is never dropped. The raw endpoint and
-- credentials are never stored.
--
-- A source whose lifetime guard is held defers its scheduling instead of
-- consuming every bounded maintenance batch, so one long index run cannot
-- starve other due sources.

ALTER TABLE context69.git_generation_code_vectors
    ADD COLUMN endpoint_identity TEXT;

ALTER TABLE context69.git_generation_code_vectors
    ADD CONSTRAINT chk_git_generation_code_vectors_endpoint_identity
        CHECK (endpoint_identity IS NULL OR btrim(endpoint_identity) <> '');

ALTER TABLE context69.git_repository_sources
    ADD COLUMN cleanup_deferred_until TIMESTAMPTZ;
