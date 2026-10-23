-- Recreates the initial state the per-target reset decision has to handle: no
-- owned schema and no shared ledger at all.
--
-- This is destructive, so a case runs it only after it has proved the target is
-- exactly the isolated fixture it owns, that the fixture still carries the owner
-- comment the parent recorded, and that the shared ledger holds nothing but the
-- rows and shape this build ships. Anything unproven refuses before this file is
-- executed.
DROP SCHEMA IF EXISTS context69 CASCADE;
DROP TABLE IF EXISTS public._sqlx_migrations;
