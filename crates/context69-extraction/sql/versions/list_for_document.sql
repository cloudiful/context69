SELECT v.id, v.document_id, v.template_key, v.template_version,
       v.source_record_hash, v.model_name, v.result_json, v.created_at
FROM context69.document_extraction_versions v
WHERE v.document_id = $1
ORDER BY v.created_at DESC

