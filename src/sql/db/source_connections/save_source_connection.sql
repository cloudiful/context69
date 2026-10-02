-- Inserts a new connection under the supplied stable identity, or updates the
-- existing one in place.
--
-- `connection_key` is deliberately absent from the update list: a connection
-- keeps the identity it was created with, so the secret-store key a DSN is
-- sealed under never moves when the connection is re-saved.
-- `database_url_secret_key` is the whole credential side of the row: the caller
-- seals the DSN under that key before calling this, because the reference has to
-- resolve to a store row that already exists. The DSN is never written here, so
-- the table holds no plaintext duplicate of it.
INSERT INTO context69.runtime_source_connections (
    connection_key,
    name,
    database_url_secret_key,
    updated_at
)
VALUES ($1, $2, $3, now())
ON CONFLICT (name) DO UPDATE
SET database_url_secret_key = EXCLUDED.database_url_secret_key,
    updated_at = now()
RETURNING name, connection_key, database_url_secret_key
