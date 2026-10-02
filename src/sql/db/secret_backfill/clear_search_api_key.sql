-- Nulls the legacy search / rerank provider API key.
--
-- Only the credential column moves: the fusion weights, the rerank base URL and
-- model, and `updated_at` all stay as they are, because a backfill is not a
-- settings change. `api_key IS NOT NULL` keeps a retry over an already-cleared
-- column a no-op, so a stopped run can simply be run again.
UPDATE context69.search_settings
SET api_key = NULL
WHERE singleton = TRUE
  AND api_key IS NOT NULL
