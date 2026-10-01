-- Inserts a new connection under the supplied stable identity, or updates the
-- existing one in place.
--
-- `connection_key` is deliberately absent from the update list: a connection
-- keeps the identity it was created with, so the secret-store key a DSN is
-- sealed under never moves when the connection is re-saved. The legacy
-- `database_url` column keeps receiving the value for the whole transition, and
-- `database_url_secret_key` records which store row now owns it.
INSERT INTO context69.runtime_source_connections (
    connection_key,
    name,
    database_url,
    database_url_secret_key,
    updated_at
)
VALUES ($1, $2, $3, $4, now())
ON CONFLICT (name) DO UPDATE
SET database_url = EXCLUDED.database_url,
    database_url_secret_key = EXCLUDED.database_url_secret_key,
    updated_at = now()
RETURNING name, connection_key, database_url, database_url_secret_key
