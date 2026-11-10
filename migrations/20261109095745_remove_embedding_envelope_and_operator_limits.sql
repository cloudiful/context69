-- Remove the embedding input envelope and the operator-configurable request and
-- concurrency limits. Only stored configuration is affected: vectors, tasks and
-- search data are untouched.
--
-- The request packing is now a fixed internal transport bound, and the URL
-- import interval is a private fixed per-origin delay.
ALTER TABLE context69.runtime_embedding_settings
    DROP COLUMN IF EXISTS max_items_per_request,
    DROP COLUMN IF EXISTS max_item_chars,
    DROP COLUMN IF EXISTS max_request_chars,
    DROP COLUMN IF EXISTS max_request_bytes,
    DROP COLUMN IF EXISTS max_input_tokens,
    DROP COLUMN IF EXISTS document_server_prefix,
    DROP COLUMN IF EXISTS query_server_prefix,
    DROP COLUMN IF EXISTS add_special_tokens,
    DROP COLUMN IF EXISTS input_tokenizer,
    DROP COLUMN IF EXISTS input_tokenizer_sha256;

ALTER TABLE context69.runtime_code_embedding_settings
    DROP CONSTRAINT IF EXISTS runtime_code_embedding_settings_batching_check,
    DROP COLUMN IF EXISTS max_items_per_request,
    DROP COLUMN IF EXISTS max_item_chars,
    DROP COLUMN IF EXISTS max_request_chars,
    DROP COLUMN IF EXISTS max_request_bytes;

ALTER TABLE context69.runtime_file_library_settings
    DROP COLUMN IF EXISTS max_upload_request_size_mb,
    DROP COLUMN IF EXISTS ingest_concurrency,
    DROP COLUMN IF EXISTS url_import_concurrency,
    DROP COLUMN IF EXISTS url_import_min_interval_ms;
