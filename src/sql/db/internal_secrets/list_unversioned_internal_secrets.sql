-- Enumerates the keys whose stored bytes are still the legacy plaintext
-- representation. This is the transition worklist for the reversible-secret
-- migration: it returns key names only, never a value, so listing what is left
-- to seal can never disclose a secret. Ordering makes the list deterministic
-- across runs.
SELECT key
FROM context69.internal_secrets
WHERE ciphertext_version = 0
ORDER BY key;
