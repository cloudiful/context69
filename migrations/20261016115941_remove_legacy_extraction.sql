-- Remove legacy extraction runtime persistence (issue #910 P2).
--
-- Drops only the four standalone built-in extraction tables and the three
-- library_files directive columns added for enrichment. No CASCADE: dependent
-- tables are dropped first so every remaining DROP is explicit. Source
-- documents, document_versions, chunks, embeddings, library_files business
-- data, task history, metadata indexes, generic document results and every
-- provider/secret/translation record are preserved.
--
-- documents.metadata_json (and its immutable document_versions snapshots) is
-- intentionally left untouched: the `extraction` key there is part of the
-- canonical record hash and is a source-document snapshot, not an active
-- result. Rewriting it would corrupt non-enrichment document integrity.

DROP TABLE context69.document_extraction_attempts;
DROP TABLE context69.document_extraction_versions;
DROP TABLE context69.document_extraction_jobs;
DROP TABLE context69.extraction_templates;

ALTER TABLE context69.library_files
    DROP COLUMN extraction_template_key,
    DROP COLUMN extraction_parameters,
    DROP COLUMN extraction_required;
