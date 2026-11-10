-- Persist the exact physical purge target of each deleted entity.
--
-- A pending tombstone must retry the endpoint and collection that its deletion
-- committed against. Resolving the current desired settings at retry time can
-- redirect the purge to a newly activated collection and falsely stamp the
-- observation while the original point remains.
--
-- Both columns are nullable: legacy rows and deletes committed without a proven
-- active index keep NULL and stay pending. The backstop trigger deliberately
-- leaves them NULL rather than inventing a current origin.

ALTER TABLE context69.library_deleted_entities
    ADD COLUMN IF NOT EXISTS target_endpoint TEXT,
    ADD COLUMN IF NOT EXISTS target_collection TEXT;

COMMENT ON COLUMN context69.library_deleted_entities.target_endpoint IS
    'Private Qdrant endpoint captured when the logical delete committed; NULL means unknown and the tombstone stays pending.';
COMMENT ON COLUMN context69.library_deleted_entities.target_collection IS
    'Physical Qdrant collection captured when the logical delete committed; NULL means unknown and the tombstone stays pending.';
