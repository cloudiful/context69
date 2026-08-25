-- Additive companion to 0023 for the legacy old-key cleanup phase. Migration
-- 0023 may already be applied in environments created before this phase, and
-- CREATE TABLE IF NOT EXISTS never alters an existing table, so the backend
-- column is added here instead. It stays nullable on purpose: rows recorded
-- before this column existed have an unknown backend, and the cleanup phase
-- must skip rather than guess. Records written after 0024 always carry the
-- active backend.
ALTER TABLE context69.library_legacy_object_cleanup
    ADD COLUMN IF NOT EXISTS old_storage_backend TEXT;

ALTER TABLE context69.library_legacy_object_cleanup
    DROP CONSTRAINT IF EXISTS library_legacy_object_cleanup_old_storage_backend_check;

ALTER TABLE context69.library_legacy_object_cleanup
    ADD CONSTRAINT library_legacy_object_cleanup_old_storage_backend_check
    CHECK (old_storage_backend IS NULL OR btrim(old_storage_backend) <> '');
