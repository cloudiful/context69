-- The persistent rebuild barrier and the captured vector identity of a
-- registry entry (issue #1138 P26).
--
-- The barrier lives in the epoch row every registry writer already locks, so
-- a full vector rebuild and every registry-admitted publication serialize on
-- one row. The barrier is durable on purpose: a node that did not observe the
-- rebuild, or that restarted while it ran, reads the same state and fails
-- closed instead of trusting an in-process flag.
--
-- The identity columns record the vector space an entry was admitted under.
-- The entry admits new vector writes, so it records the embedding fingerprint
-- and the collection it was admitted with, and the switch compares them with
-- the persisted configuration before the points become visible.
--
-- A metadata-only tombstone entry creates no point, so it carries no
-- identity.

ALTER TABLE context69.search_registry_epoch
    ADD COLUMN rebuild_state TEXT NOT NULL DEFAULT 'idle'
        CONSTRAINT chk_search_registry_epoch_rebuild_state
        CHECK (rebuild_state IN ('idle', 'running', 'failed')),
    ADD COLUMN rebuild_owner_item UUID,
    ADD COLUMN rebuild_owner_token UUID,
    ADD COLUMN rebuild_fingerprint TEXT,
    ADD COLUMN rebuild_collection TEXT,
    ADD COLUMN rebuild_started_at TIMESTAMPTZ,
    ADD COLUMN rebuild_error TEXT;

ALTER TABLE context69.search_registry_entries
    ADD COLUMN embedding_fingerprint TEXT,
    ADD COLUMN collection_name TEXT;

-- An entry that admits new vector writes records both identity fields. An
-- entry that creates no point (a metadata-only tombstone) records neither.
ALTER TABLE context69.search_registry_entries
    ADD CONSTRAINT chk_search_registry_entries_identity
    CHECK (
        (embedding_fingerprint IS NULL AND collection_name IS NULL)
        OR (embedding_fingerprint IS NOT NULL AND collection_name IS NOT NULL)
    );
