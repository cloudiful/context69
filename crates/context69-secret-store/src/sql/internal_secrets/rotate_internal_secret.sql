-- Replaces the stored representation of an existing secret in one statement, so
-- a rotation cannot leave a window in which the row is absent or half written.
-- An update that matches no row is reported as "not rotated" rather than
-- inserting, so a rotation can never conjure a secret that was never created.
UPDATE context69.internal_secrets
SET value = $2,
    purpose = $3,
    key_version = $4,
    ciphertext_version = $5,
    updated_at = NOW()
WHERE key = $1
RETURNING key;
