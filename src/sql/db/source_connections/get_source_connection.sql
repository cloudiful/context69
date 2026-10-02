-- One stored source connection, with the identity its sealed DSN is keyed by and
-- the store row that owns the value. The DSN itself is never projected.
SELECT name, connection_key, database_url_secret_key
FROM context69.runtime_source_connections
WHERE name = $1
