-- The embedding provider API key is never written here: the store is its only
-- representation, so a settings save cannot leave a plaintext duplicate behind.
--
-- The query model and the batching limits are written explicitly. A NULL
-- query_model means the document model embeds queries too.
--
-- The input policy columns are the explicit counting envelope. A NULL
-- max_input_tokens keeps the guard off; the prefixes are counted only and are
-- never sent.
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
    max_input_tokens,
    document_server_prefix,
    query_server_prefix,
    add_special_tokens,
    input_tokenizer,
    input_tokenizer_sha256,
    updated_at
)
VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, now())
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
    max_input_tokens = EXCLUDED.max_input_tokens,
    document_server_prefix = EXCLUDED.document_server_prefix,
    query_server_prefix = EXCLUDED.query_server_prefix,
    add_special_tokens = EXCLUDED.add_special_tokens,
    input_tokenizer = EXCLUDED.input_tokenizer,
    input_tokenizer_sha256 = EXCLUDED.input_tokenizer_sha256,
    updated_at = now()
