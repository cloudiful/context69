-- The embedding provider API key is never written here: the store is its only
-- representation, so a settings save cannot leave a plaintext duplicate behind.
--
-- The query model and the batching limits are written explicitly. A NULL
-- query_model means the document model embeds queries too.
INSERT INTO context69.runtime_embedding_settings (
    singleton,
    base_url,
    model,
    query_model,
    dimensions,
    timeout_secs,
    max_items_per_request,
    max_item_chars,
    max_request_chars,
    max_request_bytes,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9, now())
ON CONFLICT (singleton) DO UPDATE
SET base_url = EXCLUDED.base_url,
    model = EXCLUDED.model,
    query_model = EXCLUDED.query_model,
    dimensions = EXCLUDED.dimensions,
    timeout_secs = EXCLUDED.timeout_secs,
    max_items_per_request = EXCLUDED.max_items_per_request,
    max_item_chars = EXCLUDED.max_item_chars,
    max_request_chars = EXCLUDED.max_request_chars,
    max_request_bytes = EXCLUDED.max_request_bytes,
    updated_at = now()
