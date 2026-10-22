-- The token rows this test created.
DELETE FROM context69.personal_access_tokens
WHERE id = ANY($1::uuid[])
