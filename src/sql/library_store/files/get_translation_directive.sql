SELECT translation_override, translation_source_locale, translation_target_locales
FROM context69.library_files
WHERE id = $1
