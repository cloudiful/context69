-- Stable identity and encrypted-store reference for source connections (issue
-- #681 work unit 4A3b-3).
--
-- A source connection's database URL can carry embedded credentials, so it is a
-- reversible secret the shared encrypted store owns. The key a value is sealed
-- under has to be an identifier this application minted, which rules out both
-- alternatives a caller can influence: the connection name is user-chosen,
-- renameable, and reusable after a delete, and the DSN is the secret itself.
-- Two additive columns supply a stable one.
--
--   * `connection_key` is a stable UUID per connection. It survives a rename or
--     a re-save, so the key a DSN is sealed under keeps pointing at the same
--     place while the row is updated in place.
--   * `database_url_secret_key` is the nullable `context69.internal_secrets(key)`
--     reference, the same pattern the Git provider connections and webhook
--     registrations use. It stays NULL for a connection that has not been
--     written through the store yet, which is exactly what the transition looks
--     like: such a connection's DSN is still read from the legacy column.
--
-- Additive and rollback-safe. `database_url` is untouched and keeps receiving the
-- value, so a release that predates the store still finds what it expects, and
-- no row is sealed, backfilled, cleared, or dropped here. `name` remains the
-- primary key, so every existing lookup and query keeps working unchanged.

ALTER TABLE context69.runtime_source_connections
    ADD COLUMN connection_key UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN database_url_secret_key TEXT
        REFERENCES context69.internal_secrets(key) ON DELETE SET NULL;

-- The stable identity must be unique to be an identity: it is the value the
-- secret-store key name is derived from.
ALTER TABLE context69.runtime_source_connections
    ADD CONSTRAINT uq_runtime_source_connections_connection_key
        UNIQUE (connection_key);
