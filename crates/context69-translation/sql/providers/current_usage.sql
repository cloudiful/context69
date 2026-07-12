SELECT COALESCE(character_count, 0)::BIGINT AS "character_count!"
FROM context69.translation_provider_usage
WHERE provider_key = $1
  AND usage_month = CASE
      WHEN $1 = 'deepl' THEN DATE '1970-01-01'
      ELSE date_trunc('month', now() AT TIME ZONE 'UTC')::date
  END
