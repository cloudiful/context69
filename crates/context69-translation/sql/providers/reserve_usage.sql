INSERT INTO context69.translation_provider_usage (
    provider_key, usage_month, character_count, updated_at
)
VALUES (
    $1,
    CASE
        WHEN $1 = 'deepl' THEN DATE '1970-01-01'
        ELSE date_trunc('month', now() AT TIME ZONE 'UTC')::date
    END,
    $2,
    now()
)
ON CONFLICT (provider_key, usage_month) DO UPDATE SET
    character_count = context69.translation_provider_usage.character_count + EXCLUDED.character_count,
    updated_at = now()
WHERE $3::BIGINT IS NULL
   OR context69.translation_provider_usage.character_count + EXCLUDED.character_count <= $3
RETURNING character_count
