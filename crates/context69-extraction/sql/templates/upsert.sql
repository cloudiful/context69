INSERT INTO context69.extraction_templates (
    template_key, version, description, system_prompt, output_schema, max_output_tokens, enabled
)
VALUES ($1, $2, $3, $4, $5, $6, $7)
ON CONFLICT (template_key) DO UPDATE
SET description = EXCLUDED.description,
    system_prompt = EXCLUDED.system_prompt,
    output_schema = EXCLUDED.output_schema,
    max_output_tokens = EXCLUDED.max_output_tokens,
    enabled = EXCLUDED.enabled,
    version = EXCLUDED.version,
    updated_at = now()
RETURNING template_key, version, description, system_prompt, output_schema,
          max_output_tokens, enabled, created_at, updated_at

