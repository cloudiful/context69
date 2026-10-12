-- Where the sqlx migration ledger exists and whether its shape matches what
-- this build writes: version, description, installed_on, success, checksum,
-- execution_time. The ledger location is observed, never assumed from source.
SELECT n.nspname AS namespace,
       (SELECT count(*) = 6
          FROM pg_attribute a
          WHERE a.attrelid = c.oid
            AND a.attnum > 0
            AND NOT a.attisdropped
            AND a.attname IN ('version', 'description', 'installed_on', 'success',
                              'checksum', 'execution_time')) AS owned_shape
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relname = $1
  AND c.relkind = 'r'
ORDER BY n.nspname;