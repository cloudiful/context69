-- Project only safe classification fields; never return query text.
-- Masked statistics and prepared transactions cannot qualify as idle.
SELECT COALESCE(application_name, '') AS application_name,
       COALESCE(state, '') AS state,
       xact_start::text AS xact_start,
       COALESCE(query IS NOT NULL
                AND query <> '<insufficient privilege>'
                AND backend_start IS NOT NULL
                AND backend_type = 'client backend'
                AND state IS NOT NULL, false) AS details_visible
FROM pg_stat_activity
WHERE datname = current_database()
  AND pid <> pg_backend_pid()
  AND (backend_type = 'client backend' OR backend_type IS NULL)
  AND application_name IS DISTINCT FROM 'context69-test-runner'
UNION ALL
SELECT 'prepared transaction', 'prepared', prepared::text, false
FROM pg_prepared_xacts
WHERE database = current_database();
