-- Allow an operator-provisioned local Hugging Face tokenizer next to the
-- built-in tiktoken vocabularies. The artifact is pinned by the lowercase
-- SHA256 of its bytes; the built-in tokenizers keep a NULL hash.
ALTER TABLE context69.runtime_chunking_settings
    ADD COLUMN tokenizer_sha256 TEXT;

ALTER TABLE context69.runtime_chunking_settings
    DROP CONSTRAINT runtime_chunking_settings_tokenizer_check;

ALTER TABLE context69.runtime_chunking_settings
    ADD CONSTRAINT runtime_chunking_settings_tokenizer_check
        CHECK (tokenizer IN ('cl100k_base', 'o200k_base', 'hf_local')),
    ADD CONSTRAINT runtime_chunking_settings_tokenizer_sha256_check
        CHECK (
            (tokenizer = 'hf_local' AND tokenizer_sha256 ~ '^[0-9a-f]{64}$')
            OR (tokenizer <> 'hf_local' AND tokenizer_sha256 IS NULL)
        );
