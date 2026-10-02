-- Nulls the legacy embedding provider API key, the first of the four
-- standalone nullable credential columns a backfill may clear.
--
-- The statement is deliberately whole-column and unconditional on the stored
-- value: it runs once per category, after that category's sealed write and its
-- round trip have committed, and it touches no other column — not `base_url`,
-- not `updated_at`, and not any settings row. A row without a key matches
-- nothing, so a retry over an already-cleared column is a no-op.
UPDATE context69.runtime_embedding_settings
SET api_key = NULL
WHERE singleton = TRUE
  AND api_key IS NOT NULL
