SELECT
    (COALESCE(MAX(version), 0) + 1)::INT AS next_version
FROM context69.extraction_templates
WHERE template_key = $1

