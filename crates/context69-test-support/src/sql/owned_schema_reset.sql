-- Drop and recreate the one schema Context69 owns inside the configured test
-- database. This is the only destructive statement the runner owns.
-- Nothing outside this schema is touched: `public` is preserved, installed
-- extensions are preserved, and the database itself is preserved.
--
-- When the sqlx ledger lives inside the owned schema, dropping the schema
-- removes the ledger with it and this statement is the whole reset. When the
-- ledger lives in `public`, the verified Context69 rows there are cleared
-- separately, by `clear_verified_public_ledger.sql`.
DROP SCHEMA IF EXISTS context69 CASCADE;

CREATE SCHEMA context69;
