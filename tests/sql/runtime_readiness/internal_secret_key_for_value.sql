-- Only this test's own fixture signing key is looked up, to decide whether the
-- boot created the row it later deletes. Existing rows are never printed.
SELECT key AS "key!"
FROM context69.internal_secrets
WHERE value = $1
