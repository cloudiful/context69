WITH total AS (
    SELECT COALESCE(SUM(character_count), 0)::BIGINT AS character_count
    FROM context69.translation_provider_usage
    WHERE provider_key = 'deepl'
), cleared AS (
    DELETE FROM context69.translation_provider_usage
    WHERE provider_key = 'deepl'
)
INSERT INTO context69.translation_provider_usage (
    provider_key, usage_month, character_count, updated_at
)
SELECT 'deepl', DATE '1970-01-01', character_count, now()
FROM total
WHERE character_count > 0;
