-- Whether the relations this cleanup has to read actually exist, and the
-- database clock at observation time.
--
-- The reset runs against whatever state the owned database is in, so absence
-- must be observed before the linked-task query references a missing relation.
-- `to_regclass` resolves a missing schema or relation to NULL instead of
-- failing, which keeps a fresh or already-reset database a safe no-op.
--
-- A present library schema is a different case from an absent one: the caller
-- validates its ownership, version and required relations with the library's
-- read-only schema check before any linked selection, so a present but foreign,
-- outdated or corrupt schema refuses the reset instead of being treated as an
-- empty bootstrap.
--
-- `database_now` is the server clock the caller uses as its conservative expiry
-- anchor. Lease deadlines live on the same server clock, so a deadline at or
-- before this observation is proven expired without trusting an application
-- clock that may be skewed ahead of the database.
SELECT
    to_regclass('context69.task_items') IS NOT NULL AS "item_relation_exists!",
    EXISTS (
        SELECT 1 FROM pg_namespace WHERE nspname = 'cloudiful_tasks'
    ) AS "library_schema_exists!",
    to_regclass('cloudiful_tasks.tasks') IS NOT NULL AS "library_tasks_exists!",
    clock_timestamp() AS "database_now!";
