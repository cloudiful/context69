-- Every stored source connection, with the identity each sealed DSN is keyed by
-- and the store row that owns it. The DSN itself is never projected.
SELECT name, connection_key, database_url_secret_key
FROM context69.runtime_source_connections
ORDER BY name
