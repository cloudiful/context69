-- Removes a secret row and reports whether this statement was the one that
-- removed it, so a caller can tell a cleared secret from an absent one instead
-- of guessing. Nothing is returned but the key.
DELETE FROM context69.internal_secrets
WHERE key = $1
RETURNING key;
