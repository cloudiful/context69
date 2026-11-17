-- Personal access token group range and honest last-used telemetry.
--
-- group_scope_all = true keeps the historical "all accessible groups" range,
-- so existing tokens and old clients keep their behavior. A restricted token
-- stores the exact stable group ids; the check forbids an empty restricted set
-- so removing the last authorized group can never silently widen the token.
ALTER TABLE context69.personal_access_tokens
    ADD COLUMN group_scope_all BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN group_ids BIGINT[] NOT NULL DEFAULT '{}',
    ADD COLUMN last_used_precision TEXT,
    ADD COLUMN rotated_from_token_id UUID;

ALTER TABLE context69.personal_access_tokens
    ADD CONSTRAINT personal_access_tokens_group_scope_check
    CHECK (group_scope_all OR cardinality(group_ids) > 0);

ALTER TABLE context69.personal_access_tokens
    ADD CONSTRAINT personal_access_tokens_last_used_precision_check
    CHECK (last_used_precision IS NULL OR last_used_precision IN ('exact', 'legacy_day'));

-- Existing observations came from the coarse daily coalescing recorder. Mark
-- them honestly instead of presenting them as exact last-use instants.
UPDATE context69.personal_access_tokens
SET last_used_precision = 'legacy_day'
WHERE last_used_at IS NOT NULL;
