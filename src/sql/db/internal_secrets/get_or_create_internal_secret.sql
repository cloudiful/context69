INSERT INTO context69.internal_secrets (key, value)
VALUES ($1, $2)
ON CONFLICT (key) DO UPDATE
SET value = context69.internal_secrets.value
RETURNING value;
