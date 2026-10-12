-- One session-scoped advisory lock key pair for the backend database runner.
-- The two keys are derived from a fixed namespace text and the target
-- database's own oid, so the lock is specific to one database and two runners
-- on different databases never block each other.
SELECT hashtext('context69-backend-test-runner'),
       (SELECT oid::int4 FROM pg_database WHERE datname = current_database());