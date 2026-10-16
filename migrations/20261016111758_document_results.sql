-- Generic caller-produced document results (issue #910 P1).
--
-- Context69 stores an opaque, append-only result a caller produced from one
-- document and an exact source record hash. The row is immutable: there is no
-- update path, and the same idempotency key under the same document, author,
-- and producer replays the original row instead of appending a second one.
--
-- Ownership is the authenticated `author_user_id`, never a caller-supplied
-- producer string. The author reference is ON DELETE RESTRICT so removing a
-- user cannot silently discard the result provenance they produced; the
-- document reference keeps the existing document-owned data deletion semantics
-- (CASCADE), matching every other document-owned table.

CREATE TABLE context69.document_results (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    author_user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE RESTRICT,
    producer TEXT NOT NULL,
    result_kind TEXT NOT NULL,
    schema_ref TEXT NOT NULL,
    source_record_hash TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    result JSONB NOT NULL,
    provenance JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_document_results_result_object
        CHECK (jsonb_typeof(result) = 'object'),
    CONSTRAINT chk_document_results_provenance_object
        CHECK (provenance IS NULL OR jsonb_typeof(provenance) = 'object')
);

-- One idempotency key means one immutable result per (document, author,
-- producer). Distinct runs use distinct keys, so append history is retained.
CREATE UNIQUE INDEX uq_document_results_identity
    ON context69.document_results (document_id, author_user_id, producer, idempotency_key);

-- The list reads one document newest-first.
CREATE INDEX idx_document_results_document_id_id
    ON context69.document_results (document_id, id DESC);
