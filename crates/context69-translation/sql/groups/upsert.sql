INSERT INTO context69.group_translation_settings (
    group_id, enabled, default_target_locales, source_locale, glossary, updated_at
)
VALUES ($1, $2, $3, $4, $5, now())
ON CONFLICT (group_id) DO UPDATE SET
    enabled = EXCLUDED.enabled,
    default_target_locales = EXCLUDED.default_target_locales,
    source_locale = EXCLUDED.source_locale,
    glossary = EXCLUDED.glossary,
    updated_at = now()
RETURNING enabled, default_target_locales, source_locale, glossary
