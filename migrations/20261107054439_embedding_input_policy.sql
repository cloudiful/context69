-- The explicit embedding input-token envelope.
--
-- These columns describe how the provider counts a full input. They never
-- change the text that is sent: the two prefixes are added by the provider
-- itself and are counted only. A NULL max_input_tokens keeps the guard off.
ALTER TABLE context69.runtime_embedding_settings
    ADD COLUMN max_input_tokens BIGINT,
    ADD COLUMN document_server_prefix TEXT NOT NULL DEFAULT '',
    ADD COLUMN query_server_prefix TEXT NOT NULL DEFAULT '',
    ADD COLUMN add_special_tokens BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN input_tokenizer TEXT NOT NULL DEFAULT 'cl100k_base',
    ADD COLUMN input_tokenizer_sha256 TEXT;

ALTER TABLE context69.runtime_embedding_settings
    ADD CONSTRAINT runtime_embedding_settings_max_input_tokens_check
        CHECK (max_input_tokens IS NULL OR max_input_tokens > 0),
    ADD CONSTRAINT runtime_embedding_settings_input_tokenizer_check
        CHECK (input_tokenizer IN ('cl100k_base', 'o200k_base', 'hf_local')),
    ADD CONSTRAINT runtime_embedding_settings_input_tokenizer_sha256_check
        CHECK (
            (input_tokenizer = 'hf_local' AND input_tokenizer_sha256 ~ '^[0-9a-f]{64}$')
            OR (input_tokenizer <> 'hf_local' AND input_tokenizer_sha256 IS NULL)
        ),
    ADD CONSTRAINT runtime_embedding_settings_add_special_tokens_check
        CHECK (input_tokenizer = 'hf_local' OR add_special_tokens = FALSE);
