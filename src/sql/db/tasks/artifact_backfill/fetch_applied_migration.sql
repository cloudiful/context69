-- Applied ledger entry for the #975 migration validation (issue #975 P5).
--
-- The apply path compares this checksum with the checksum compiled into this
-- binary and refuses to continue on a missing, failed or different entry. No
-- DDL and no ledger write happens here.
SELECT checksum,
       success
FROM _sqlx_migrations
WHERE version = $1
