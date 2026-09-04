-- Persistent Docling remote admission limit (issue #118 phase 2).
--
-- Mac mini runs Docling Serve in RQ mode with a single RQ worker, so the
-- safe initial ceiling for concurrent remote (non-terminal) jobs is 1.
-- The limit is adjustable via settings but bounded to prevent unbounded
-- submission floods. Existing rows backfill to 1.
ALTER TABLE context69.docling_settings
    ADD COLUMN IF NOT EXISTS max_inflight BIGINT NOT NULL DEFAULT 1
    CHECK (max_inflight >= 1 AND max_inflight <= 32);
