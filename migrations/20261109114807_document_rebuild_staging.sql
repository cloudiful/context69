-- Fresh document-rechunk rebuild staging (issue #27 P2).
--
-- A rebuild now re-chunks every current document from its canonical
-- `document_versions.body_text` under a budget pinned at begin, and replays the
-- current translated chunks beside the fresh originals. The staged rows are
-- keyed by the physical target collection, so a resumed attempt of the same
-- target keeps them and a fresh or discarded target never inherits them.
--
-- The pinned budget, chunk-id seed, staging completion flag and the two
-- independent paging cursors live on the same epoch row as the barrier, so the
-- ownership decision and the committed progress remain one durable record.

ALTER TABLE context69.search_registry_epoch
    ADD COLUMN rebuild_chunk_budget JSONB,
    ADD COLUMN rebuild_generation BIGINT,
    ADD COLUMN rebuild_staging_complete BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN rebuild_staged_document_id BIGINT,
    ADD COLUMN rebuild_translation_cursor_document_id BIGINT,
    ADD COLUMN rebuild_translation_cursor_translation_id UUID,
    ADD COLUMN rebuild_translation_cursor_chunk_index INTEGER,
    ADD COLUMN rebuild_translation_processed_chunks BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_generation
        CHECK (rebuild_generation IS NULL OR rebuild_generation >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_staged_document
        CHECK (rebuild_staged_document_id IS NULL OR rebuild_staged_document_id >= 0),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_translation_cursor
        CHECK (
            (rebuild_translation_cursor_document_id IS NULL)
                = (rebuild_translation_cursor_translation_id IS NULL)
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_translation_cursor_index
        CHECK (
            (rebuild_translation_cursor_document_id IS NULL)
                = (rebuild_translation_cursor_chunk_index IS NULL)
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_translation_cursor_index_value
        CHECK (
            rebuild_translation_cursor_chunk_index IS NULL
                OR rebuild_translation_cursor_chunk_index >= 0
        ),
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_translation_processed
        CHECK (rebuild_translation_processed_chunks >= 0);

CREATE TABLE IF NOT EXISTS context69.document_rebuild_staging_documents (
    target_collection TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    record_hash TEXT NOT NULL,
    chunk_count INTEGER NOT NULL,
    PRIMARY KEY (target_collection, document_id),
    CONSTRAINT chk_document_rebuild_staging_documents_chunk_count
        CHECK (chunk_count >= 0)
);

CREATE TABLE IF NOT EXISTS context69.document_chunk_rebuild_staging (
    target_collection TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    chunk_index INTEGER NOT NULL,
    chunk_id UUID NOT NULL,
    chunk_text TEXT NOT NULL,
    record_hash TEXT NOT NULL,
    PRIMARY KEY (target_collection, document_id, chunk_index),
    CONSTRAINT chk_document_chunk_rebuild_staging_index CHECK (chunk_index >= 0)
);

CREATE TABLE IF NOT EXISTS context69.document_translation_rebuild_staging (
    target_collection TEXT NOT NULL,
    translation_id UUID NOT NULL,
    document_id BIGINT NOT NULL,
    chunk_index INTEGER NOT NULL,
    chunk_id UUID NOT NULL,
    chunk_text TEXT NOT NULL,
    record_hash TEXT NOT NULL,
    PRIMARY KEY (target_collection, translation_id, chunk_index),
    CONSTRAINT chk_document_translation_rebuild_staging_index CHECK (chunk_index >= 0)
);
