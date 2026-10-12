-- Whether the schema this runner owns already exists in the target database.
SELECT EXISTS (
    SELECT 1
    FROM pg_namespace
    WHERE nspname = $1
);