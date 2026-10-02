-- Nulls the legacy Docling VLM provider API key.
--
-- One column only. The base URL, the model names, and the OCR/representation
-- flags are configuration rather than credentials and keep their values, so a
-- deployment that has not re-entered its VLM key is not silently reconfigured
-- by a backfill. `api_key IS NOT NULL` makes a repeated run a no-op.
UPDATE context69.docling_settings
SET api_key = NULL
WHERE singleton = TRUE
  AND api_key IS NOT NULL
