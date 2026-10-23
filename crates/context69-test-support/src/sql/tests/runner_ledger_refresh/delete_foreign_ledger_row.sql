-- Removes only the shared-ledger row this case injected, so a refusal case
-- leaves the fixture as it found it.
DELETE FROM public._sqlx_migrations WHERE version = $1
