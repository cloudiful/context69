-- Points one source connection at the sealed database URL that now owns its
-- value.
--
-- The broad save statement is deliberately not reused: it rewrites
-- `database_url`, and the backfill must leave the legacy DSN exactly as it is
-- until the column-removal migration, because that column is `NOT NULL` with a
-- non-blank check and nulling it here would fail the constraint. This moves
-- `database_url_secret_key` and `updated_at` only, which leaves the stable
-- `connection_key` the sealed value is keyed by untouched — repointing a
-- connection must never move it onto a new identity.
--
-- Scoped by name, the user-facing identifier, because source connections are
-- not group-owned. A name that matches no row updates nothing, and the caller
-- learns the reference did not land instead of believing it did.
UPDATE context69.runtime_source_connections
SET database_url_secret_key = $2,
    updated_at = now()
WHERE name = $1
