INSERT INTO context69.document_extraction_versions (
    id, document_id, template_key, template_version, source_record_hash,
    provider_key, provider_config_hash, model_name, result_json
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)

