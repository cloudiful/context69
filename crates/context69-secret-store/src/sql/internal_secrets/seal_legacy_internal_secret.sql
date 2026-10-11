-- Converts one claimed legacy plaintext row in a single statement, so a row can
-- never be observed half written and its plaintext value is replaced
-- atomically.
--
-- The markers are both set and re-asserted in the predicate. The statement is
-- guarded by the exact legacy tuple it converts, so a row that a concurrent
-- writer has already sealed is reported as "not converted" and the batch is
-- aborted rather than this statement overwriting a newer value.
--
-- `key`, `created_at` and `updated_at` are deliberately absent from the SET
-- list. The key is the row's identity and every reference to it stays valid;
-- `updated_at` records when a *value* was written through the store, and a
-- representation change is not one, so a legacy row's NULL stays NULL.
UPDATE context69.internal_secrets
SET value = $2,
    purpose = $3,
    key_version = $4,
    ciphertext_version = $5
WHERE key = $1
  AND ciphertext_version = 0
  AND purpose IS NULL
  AND key_version = 0
RETURNING key;