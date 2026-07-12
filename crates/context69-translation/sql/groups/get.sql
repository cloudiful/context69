SELECT enabled, default_target_locales, source_locale, glossary
FROM context69.group_translation_settings
WHERE group_id = $1
