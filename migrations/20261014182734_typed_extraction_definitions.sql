-- Typed extraction/decision template definitions and immutable result-kind
-- storage (issue #89 P4).
--
-- Public template fields become a tagged definition (extraction vs decision)
-- while persisted legacy rows keep their exact generative columns: existing
-- rows default to extraction and execute byte-identically. Decision rows
-- store inert generative columns (empty prompt, empty object schema, default
-- cap) only to satisfy the retained NOT NULL/CHECK constraints; dispatch
-- never treats those as generative. Result rows snapshot their kind at
-- publish time so a later template-kind change cannot reinterpret history.
-- Legacy version rows default to extraction (generative decode).

ALTER TABLE context69.extraction_templates
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'extraction',
    ADD COLUMN decision_definition JSONB NULL;

ALTER TABLE context69.extraction_templates
    ADD CONSTRAINT chk_extraction_template_kind
        CHECK (kind IN ('extraction', 'decision'));

ALTER TABLE context69.extraction_templates
    ADD CONSTRAINT chk_extraction_template_decision_object
        CHECK (decision_definition IS NULL OR jsonb_typeof(decision_definition) = 'object');

ALTER TABLE context69.document_extraction_versions
    ADD COLUMN template_kind TEXT NOT NULL DEFAULT 'extraction';

ALTER TABLE context69.document_extraction_versions
    ADD CONSTRAINT chk_document_extraction_version_kind
        CHECK (template_kind IN ('extraction', 'decision'));
