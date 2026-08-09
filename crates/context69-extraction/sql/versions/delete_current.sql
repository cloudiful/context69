DELETE FROM context69.document_extraction_versions
WHERE document_id = $1 AND template_key = $2 AND template_version = $3
  AND source_record_hash = $4

