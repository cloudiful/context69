-- Record the producer-proven original point ids of a replacement switch.
--
-- A switched replacement entry retires the old original chunks and every
-- stale translation of the affected documents. Only the original subset is
-- proven by the producer query that reads document_chunks before the delete;
-- the translation subset is not recoverable afterwards because a translation
-- may be republished or removed.
--
-- The quota projection discounts the original subset exactly when the entry
-- belongs to a physical collection other than the active one, because those
-- canonical rows no longer exist and a rebuild has recreated them under its
-- own reservation. A legacy row keeps the empty default and is counted in
-- full; nothing is inferred from missing rows.
ALTER TABLE context69.search_registry_entries
    ADD COLUMN retired_original_ids UUID[] NOT NULL DEFAULT '{}'::uuid[],
    ADD CONSTRAINT chk_search_registry_entries_retired_origin
        CHECK (retired_original_ids <@ retired_ids);

COMMENT ON COLUMN context69.search_registry_entries.retired_original_ids IS
    'Producer-proven old original point ids retired by a replacement switch; always a subset of retired_ids.';
