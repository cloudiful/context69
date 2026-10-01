-- Reads one secret row exactly as stored: the version markers, the owning
-- purpose, and the value bytes. The caller already knows the key it asked for,
-- so the key is not projected back. The store decides whether these bytes are a
-- frame it can open or a legacy plaintext value it must hand back unchanged.
SELECT value,
       purpose,
       key_version,
       ciphertext_version
FROM context69.internal_secrets
WHERE key = $1;
