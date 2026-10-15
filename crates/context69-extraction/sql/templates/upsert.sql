INSERT INTO context69.extraction_templates (
    template_key, version, description, system_prompt, output_schema, max_output_tokens, enabled,
    kind, decision_definition
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
ON CONFLICT (template_key) DO UPDATE
SET description = EXCLUDED.description,
    system_prompt = EXCLUDED.system_prompt,
    output_schema = EXCLUDED.output_schema,
    max_output_tokens = EXCLUDED.max_output_tokens,
    enabled = EXCLUDED.enabled,
    version = EXCLUDED.version,
    kind = EXCLUDED.kind,
    decision_definition = EXCLUDED.decision_definition,
    updated_at = now()
RETURNING template_key, version, description, system_prompt, output_schema,
          max_output_tokens, enabled, kind, decision_definition, created_at, updated_at
