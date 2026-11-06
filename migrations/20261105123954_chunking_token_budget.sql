-- Explicit chunking budget of the runtime settings (issue 1206 P2).
--
-- The row carried a character budget only, so a deployment could not express a
-- token budget at all. The unit, the tokenizer and the token budget join the
-- character budget in the same row: the character values keep their own
-- meaning, and the new columns default to the character budget, so a row that
-- was written before this migration keeps describing the same chunks.
--
-- The defaults are the ones the settings contract publishes, so a client that
-- does not know the token budget still reads a valid one. The constraints
-- refuse a budget that no chunker could honor.
ALTER TABLE context69.runtime_chunking_settings
    ADD COLUMN unit TEXT NOT NULL DEFAULT 'chars',
    ADD COLUMN tokenizer TEXT NOT NULL DEFAULT 'cl100k_base',
    ADD COLUMN max_tokens BIGINT NOT NULL DEFAULT 1024,
    ADD COLUMN overlap_tokens BIGINT NOT NULL DEFAULT 128;

ALTER TABLE context69.runtime_chunking_settings
    ADD CONSTRAINT runtime_chunking_settings_unit_check CHECK (unit IN ('chars', 'tokens')),
    ADD CONSTRAINT runtime_chunking_settings_tokenizer_check
        CHECK (tokenizer IN ('cl100k_base', 'o200k_base')),
    ADD CONSTRAINT runtime_chunking_settings_max_tokens_check CHECK (max_tokens > 0),
    ADD CONSTRAINT runtime_chunking_settings_overlap_tokens_check
        CHECK (overlap_tokens >= 0 AND overlap_tokens < max_tokens);
