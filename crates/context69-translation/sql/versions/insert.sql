INSERT INTO context69.document_translation_versions (
    id, document_id, target_locale, source_locale, source_record_hash,
    provider_key, provider_config_hash, model_name, translated_title,
    translated_summary, translated_body_text
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
