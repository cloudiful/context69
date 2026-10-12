-- Extensions installed inside the owned schema. Dropping the schema would
-- remove them, which the runner never does.
SELECT e.extname
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE n.nspname = $1
ORDER BY e.extname;