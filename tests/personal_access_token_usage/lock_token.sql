-- One token row, held by the test's own transaction.
--
-- The lock is what makes a telemetry write slow: the guarded batch waits for
-- the row, while the authorization read stays free. The transaction is always
-- rolled back, so the row is never left locked.
SELECT id
FROM context69.personal_access_tokens
WHERE id = $1
FOR UPDATE
