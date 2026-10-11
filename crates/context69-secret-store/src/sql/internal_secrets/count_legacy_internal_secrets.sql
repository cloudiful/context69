-- How many rows still hold the legacy plaintext representation.
--
-- The sweep reports success only when this is zero, so a row the claim query
-- could not classify can never be counted as converted. The filter is the
-- representation marker alone, with no key-name or marker refinement: a row
-- left behind for any reason still shows up here.
SELECT count(*)
FROM context69.internal_secrets
WHERE ciphertext_version = 0;