-- Desired configuration versus one active vector index (issue 1212 P4).
--
-- The runtime settings describe what the deployment is configured to write,
-- while the active row names the one physical collection that ordinary point
-- writers may use. Keeping them apart lets an operator save a new desired
-- dimension or model while the old active collection still serves queries, and
-- an explicit rebuild later activates a fresh target.
--
-- The active pointer is unique on purpose: at most one physical collection is
-- live. A row that predates this migration is active only when it is the
-- collection the persisted runtime settings name, so an installation whose
-- settings do not match any recorded identity fails closed instead of adopting
-- an arbitrary collection.
ALTER TABLE context69.vector_index_state
    ADD COLUMN active BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE context69.vector_index_state AS state
SET active = TRUE
FROM context69.runtime_qdrant_settings AS settings
WHERE settings.singleton = TRUE
  AND state.collection_name = settings.collection_name;

CREATE UNIQUE INDEX vector_index_state_one_active
    ON context69.vector_index_state (active)
    WHERE active;
