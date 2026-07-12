UPDATE context69.translation_provider_settings
SET endpoint = CASE deepl_plan
    WHEN 'pro' THEN 'https://api.deepl.com'
    ELSE 'https://api-free.deepl.com'
END,
updated_at = now()
WHERE provider_key = 'deepl'
  AND (endpoint IS NULL OR btrim(endpoint) = '');
