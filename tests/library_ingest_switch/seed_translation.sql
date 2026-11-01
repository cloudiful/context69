INSERT INTO context69.document_translation_versions
    (id, document_id, target_locale, source_record_hash, provider_key, provider_config_hash, translated_title, translated_body_text)
VALUES ($1, $2, $3, $4, 'test', 'test', 'test', 'test')
