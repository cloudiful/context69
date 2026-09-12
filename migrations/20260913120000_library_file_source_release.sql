-- Source-file lifecycle (issue 332 phase 2): immutable upload-time opt-in and
-- a persisted distinction between a deliberate source release and an
-- incidentally missing source.
--
-- `delete_source_after_processing` is chosen once at upload and is never
-- retroactively changed by a dedup/reuse path.
-- `source_released_at` records a deliberate release (manual or auto). It lets
-- the missing-source cleanup tell "the bytes are gone on purpose" apart from
-- "the bytes are gone", so a released file's derived text/vectors are kept.
ALTER TABLE context69.library_files
    ADD COLUMN IF NOT EXISTS delete_source_after_processing BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS source_released_at TIMESTAMPTZ;

-- Auto-release retry scans only opted-in, not-yet-released rows.
CREATE INDEX IF NOT EXISTS idx_library_files_pending_source_release
    ON context69.library_files (id)
    WHERE delete_source_after_processing
      AND source_released_at IS NULL;
