-- The code embedding provider API key is not projected: the shared encrypted
-- store owns it under its own purpose, independent of the document key.
--
-- Every non-secret field is nullable so an explicitly disabled row can retain
-- whatever the operator stored. A NULL query model means the code model embeds
-- queries too. A row that predates the batching limits is read with the
-- published defaults.
SELECT enabled,
       base_url,
       model,
       query_model,
       dimensions,
       timeout_secs,
       max_items_per_request,
       max_item_chars,
       max_request_chars,
       max_request_bytes
FROM context69.runtime_code_embedding_settings
WHERE singleton = TRUE
