-- Durable per-file latest-state marker for the canonical metadata mirror
-- (issue #27 P8).
--
-- A post-commit payload send cannot be part of the metadata transaction, and
-- two sends of the same file may reach the vector index out of commit order.
-- The marker is not a payload copy: it names the file whose canonical payloads
-- should be mirrored and a version that every enqueue advances. The mirror
-- worker rereads the current canonical payloads under a per-file session lock
-- and completes only that exact version, so a stale send can never erase a
-- newer publication.
--
-- No foreign key, history, lease, collection or configuration is recorded: a
-- deleted file simply produces no canonical payloads, and the rebuild barrier
-- keeps the active pointer authoritative.

CREATE TABLE context69.library_file_metadata_mirror (
    file_id UUID PRIMARY KEY,
    version BIGINT NOT NULL CHECK (version > 0),
    attempts INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX library_file_metadata_mirror_due_idx
    ON context69.library_file_metadata_mirror (next_attempt_at, file_id);

COMMENT ON TABLE context69.library_file_metadata_mirror IS
    'One durable latest-state marker per library file whose canonical metadata still needs a vector payload mirror.';
