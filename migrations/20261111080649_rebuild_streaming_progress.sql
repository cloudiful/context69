-- Per-document streaming rebuild progress (issue #31 P3).
--
-- A streaming rebuild stages and embeds one document at a time, so its
-- committed original counter advances while the final staged total is still
-- unknown. `rebuild_total_chunks = 0` therefore means "unknown" only while
-- `rebuild_staging_complete` is false; `complete_rebuild_staging` still writes
-- the exact total, and the pre-activation verification is unchanged. An empty
-- library settles as 0/0, which reads identically under either branch.
ALTER TABLE context69.search_registry_epoch
    DROP CONSTRAINT chk_search_registry_epoch_rebuild_progress,
    ADD CONSTRAINT chk_search_registry_epoch_rebuild_progress
        CHECK (
            (NOT rebuild_staging_complete AND rebuild_total_chunks = 0)
                OR rebuild_processed_chunks <= rebuild_total_chunks
        );

-- One document's current translations are drained in chunk order after its
-- originals, so the page query needs the document leading the translation
-- cursor instead of the translation version alone.
CREATE INDEX IF NOT EXISTS idx_document_translation_rebuild_staging_document
    ON context69.document_translation_rebuild_staging (
        target_collection,
        document_id,
        translation_id,
        chunk_index
    );
