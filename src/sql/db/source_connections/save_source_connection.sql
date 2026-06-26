INSERT INTO context69.runtime_source_connections (
    name,
    database_url,
    updated_at
)
VALUES ($1, $2, now())
ON CONFLICT (name) DO UPDATE
SET database_url = EXCLUDED.database_url,
    updated_at = now()
RETURNING name, database_url
