-- Creates a secret row. Deliberately not an upsert: a create that silently
-- replaced an existing value would make "put" indistinguishable from
-- "rotate", and the plan requires those to stay separate operations. Returns
-- the key when this statement created the row and no row when the key was
-- already taken, so the caller can re-read instead of assuming it won.
INSERT INTO context69.internal_secrets (key, value, purpose, key_version, ciphertext_version, updated_at)
VALUES ($1, $2, $3, $4, $5, NOW())
ON CONFLICT (key) DO NOTHING
RETURNING key;
