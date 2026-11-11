-- Durable post-activation retirement of replaced document vector collections.
--
-- A verified rebuild atomically activates its fresh target. The previous active
-- physical collection is no longer reachable from the application: startup
-- attaches only the recorded active pointer, new targets always carry a fresh
-- name, and settings never reactivate a historical name. This migration records
-- the Qdrant endpoint of each identity so a later retirement deletes on the
-- endpoint that identity was actually saved with, and adds the small pending
-- queue the background maintenance drains.
--
-- `qdrant_url` stays nullable and is never backfilled: a legacy row carries no
-- endpoint, and only a complete fingerprint reconstruction can prove one. The
-- retirement queue keeps no completed history; draining a row deletes it.

ALTER TABLE context69.vector_index_state
    ADD COLUMN IF NOT EXISTS qdrant_url TEXT;

COMMENT ON COLUMN context69.vector_index_state.qdrant_url IS
    'Private Qdrant endpoint captured when the identity was saved; NULL on legacy rows, whose endpoint is never inferred.';

CREATE TABLE IF NOT EXISTS context69.vector_collection_retirement (
    id BIGSERIAL PRIMARY KEY,
    endpoint TEXT NOT NULL CHECK (endpoint <> ''),
    collection_name TEXT NOT NULL CHECK (collection_name <> ''),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (endpoint, collection_name)
);

COMMENT ON TABLE context69.vector_collection_retirement IS
    'Pending physical Qdrant collections replaced by a successful verified activation. A drained row is deleted; no completed history is kept.';

COMMENT ON COLUMN context69.vector_collection_retirement.endpoint IS
    'Pinned private Qdrant endpoint that owns the retired collection; never resolved from current settings.';

CREATE INDEX IF NOT EXISTS vector_collection_retirement_due_idx
    ON context69.vector_collection_retirement (next_attempt_at, id);
