UPDATE context69.library_files
SET translation_override = $2,
    translation_source_locale = $3,
    translation_target_locales = $4,
    updated_at = now()
WHERE id = $1
