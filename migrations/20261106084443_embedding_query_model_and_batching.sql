-- Optional query model and manual request limits of the embedding provider
-- (issue 1212 P1).
--
-- The provider gained an optional second model for queries and one request
-- planner for every embedding call. The document model stays the document
-- model: a NULL query_model means queries are embedded with it, so a row
-- written before this migration keeps describing the same provider.
--
-- The batching columns are NULL for a row that predates them. A read maps NULL
-- to the published defaults, and a save writes the effective limits
-- explicitly, so the planner never has to guess a limit.
ALTER TABLE context69.runtime_embedding_settings
    ADD COLUMN query_model TEXT,
    ADD COLUMN max_items_per_request BIGINT,
    ADD COLUMN max_item_chars BIGINT,
    ADD COLUMN max_request_chars BIGINT,
    ADD COLUMN max_request_bytes BIGINT;

ALTER TABLE context69.runtime_embedding_settings
    ADD CONSTRAINT runtime_embedding_settings_query_model_check
        CHECK (query_model IS NULL OR btrim(query_model) <> ''),
    ADD CONSTRAINT runtime_embedding_settings_max_items_per_request_check
        CHECK (max_items_per_request IS NULL OR max_items_per_request > 0),
    ADD CONSTRAINT runtime_embedding_settings_max_item_chars_check
        CHECK (max_item_chars IS NULL OR max_item_chars > 0),
    ADD CONSTRAINT runtime_embedding_settings_max_request_chars_check
        CHECK (max_request_chars IS NULL OR max_request_chars > 0),
    ADD CONSTRAINT runtime_embedding_settings_max_request_bytes_check
        CHECK (max_request_bytes IS NULL OR max_request_bytes > 0);
