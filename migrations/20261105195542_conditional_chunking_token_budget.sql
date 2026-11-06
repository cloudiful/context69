-- Inactive token budgets stay inert in character mode (issue 1206 R2).
--
-- The token rules describe the budget token mode actually uses. While the unit
-- is `chars`, the token values are inactive and a character save persists them
-- unchanged, so the token-only constraints must not refuse that save after the
-- settings transaction has already applied its secret patches.
--
-- The unit and tokenizer vocabulary stay unconditional, the overlap stays a
-- non-negative count in both modes, and a token-mode budget keeps the existing
-- positive/overlap rules. A row written before this migration satisfies the new
-- constraints unchanged.
ALTER TABLE context69.runtime_chunking_settings
    DROP CONSTRAINT runtime_chunking_settings_max_tokens_check,
    DROP CONSTRAINT runtime_chunking_settings_overlap_tokens_check;

ALTER TABLE context69.runtime_chunking_settings
    ADD CONSTRAINT runtime_chunking_settings_max_tokens_check
        CHECK (unit <> 'tokens' OR max_tokens > 0),
    ADD CONSTRAINT runtime_chunking_settings_overlap_tokens_check
        CHECK (overlap_tokens >= 0 AND (unit <> 'tokens' OR overlap_tokens < max_tokens));
