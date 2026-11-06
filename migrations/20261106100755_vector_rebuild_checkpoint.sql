-- Durable rebuild checkpoint, pause request and cursor (issue 1212 P3).
--
-- A vector rebuild is a long operation that must survive a restart and must
-- resume after the last successfully written page instead of page zero. The
-- cursor and the counters live on the same epoch row as the barrier, so the
-- ownership decision and the committed progress are one durable record.
--
-- The cursor is paired on purpose: either both coordinates are recorded or the
-- rebuild starts from the beginning. A pause flag is only meaningful while the
-- barrier is running, so the state constraint keeps a paused row from
-- describing an idle or failed rebuild.
ALTER TABLE context69.search_registry_epoch
    ADD COLUMN rebuild_pause_requested BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN rebuild_cursor_document_id BIGINT,
    ADD COLUMN rebuild_cursor_chunk_index INTEGER,
    ADD COLUMN rebuild_processed_chunks BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN rebuild_total_chunks BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_cursor
        CHECK (
            (rebuild_cursor_document_id IS NULL) = (rebuild_cursor_chunk_index IS NULL)
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_cursor_index
        CHECK (rebuild_cursor_chunk_index IS NULL OR rebuild_cursor_chunk_index >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_processed
        CHECK (rebuild_processed_chunks >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_total
        CHECK (rebuild_total_chunks >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_progress
        CHECK (rebuild_processed_chunks <= rebuild_total_chunks),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_pause
        CHECK (NOT rebuild_pause_requested OR rebuild_state = 'running');
