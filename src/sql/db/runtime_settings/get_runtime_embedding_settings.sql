-- The embedding provider API key is not projected: the shared encrypted store
-- owns it, and the settings service resolves it through that store alone.
--
-- The query model is optional: NULL means the document model embeds queries
-- too.
SELECT base_url,
       model,
       query_model,
       dimensions,
       timeout_secs
FROM context69.runtime_embedding_settings
WHERE singleton = TRUE
