-- Direct, resumable derived-index installation (issue 44 P3).
--
-- The install checkpoints live on the same epoch row as the rebuild barrier, so
-- ownership, derived-index validity and the committed install position are one
-- durable record. `document_index_invalid` is durable derived-index validity: it
-- is set before the first destructive original-registry mutation of a rebuild
-- and cleared only by a successful publish, so a paused, failed, cancelled or
-- superseded rebuild can never make a partially installed chunk table
-- searchable. A never-rebuilt or keyword-only deployment keeps the default
-- `false` and stays valid.
--
-- `document_rebuild_staging_cleanup` is the staging-cleanup address. Completion
-- and cancellation enqueue only their own target, and the existing maintenance
-- tick drains it under the existing epoch lock. Remaining staging rows are the
-- progress, so no lease, retry counter, timer or job framework is added.

ALTER TABLE context69.search_registry_epoch
    ADD COLUMN document_index_invalid BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN rebuild_install_started BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN rebuild_install_cursor_document_id BIGINT,
    ADD COLUMN rebuild_install_cursor_chunk_index INTEGER,
    ADD COLUMN rebuild_installed_chunks BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN rebuild_install_total_chunks BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_install_cursor
        CHECK (
            (rebuild_install_cursor_document_id IS NULL)
                = (rebuild_install_cursor_chunk_index IS NULL)
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_install_cursor_index
        CHECK (
            rebuild_install_cursor_chunk_index IS NULL
                OR rebuild_install_cursor_chunk_index >= 0
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_installed_chunks
        CHECK (rebuild_installed_chunks >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_install_total_chunks
        CHECK (rebuild_install_total_chunks >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_installed_bound
        CHECK (rebuild_installed_chunks <= rebuild_install_total_chunks);

CREATE TABLE IF NOT EXISTS context69.document_rebuild_staging_cleanup (
    target_collection TEXT PRIMARY KEY
);
