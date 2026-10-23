-- The usage timestamp of one token, written by the test itself.
--
-- The guard under test reads `last_used_at`, so a case needs to place a row in
-- a chosen state before the guarded batch runs.
UPDATE context69.personal_access_tokens
SET last_used_at = $2
WHERE id = $1
