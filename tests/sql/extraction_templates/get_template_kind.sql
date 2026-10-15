-- Static branch-column readback for typed template registry tests
-- (tests/extraction_templates.rs). $1 is the template key.
SELECT template_key, version, kind, decision_definition
FROM context69.extraction_templates
WHERE template_key = $1
