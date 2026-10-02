SELECT purpose, ciphertext_version
FROM context69.internal_secrets
WHERE key = $1;
