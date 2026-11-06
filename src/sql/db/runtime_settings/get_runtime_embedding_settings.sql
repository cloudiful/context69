-- The embedding provider API key is not projected: the shared encrypted store
-- owns it, and the settings service resolves it through that store alone.
--
-- The query model is optional: NULL means the document model embeds queries
-- too. The batching limits are optional for the same reason: a row that
-- predates them is read with the published defaults.
SELECT base_url,
       model,
       query_model,
       dimensions,
       timeout_secs,
       max_items_per_request,
       max_item_chars,
       max_request_chars,
       max_request_bytes
FROM context69.runtime_embedding_settings
WHERE singleton = TRUE
