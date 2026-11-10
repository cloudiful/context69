-- The code embedding provider API key is not projected: the shared encrypted
-- store owns it under its own purpose, independent of the document key.
--
-- Every non-secret field is nullable so an explicitly disabled row can retain
-- whatever the operator stored. A NULL query model means the code model embeds
-- queries too.
SELECT enabled,
       base_url,
       model,
       query_model,
       dimensions,
       timeout_secs
FROM context69.runtime_code_embedding_settings
WHERE singleton = TRUE
