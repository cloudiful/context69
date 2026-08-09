SELECT template_key, version, description, system_prompt, output_schema, max_output_tokens,
       enabled, created_at, updated_at
FROM context69.extraction_templates
WHERE template_key = $1

