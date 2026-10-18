-- Retire embedded enrichment on normal version rollout (issue #910 S1).
--
-- One ordinary transactional migration. It:
--
--   * appends an opaque imported result for each known embedded enrichment
--     payload that still lives in `documents.metadata_json`,
--   * removes that payload from the source metadata while keeping the exact
--     existing source generation (record hash, canonical snapshot, chunks,
--     vectors, translations) untouched,
--   * clears the obsolete `failure_stage = 'extraction'` label and removes the
--     retired metadata-index definitions even when no document still carries
--     the key, and
--   * enqueues a durable, domain-neutral outbox row so the background worker
--     converges the external payload mirror.
--
-- Guarding is fail-closed and all-or-nothing: any affected document whose
-- source cannot be proven safe raises, and the whole transaction rolls back.
-- The canonical `document_versions` snapshot is the source authority; the
-- migration never recomputes or rewrites a record hash, never joins chunks into
-- a body, and never deletes history. A database without affected data (such as
-- a fresh deployment) passes the document guards without needing a bootstrap
-- admin while still converging the obsolete labels and index definitions.

-- Domain-neutral payload-mirror outbox. Holds only the document identity, the
-- fixed payload key, and lease/retry/completion state: never the old payload
-- itself.
CREATE TABLE IF NOT EXISTS context69.document_metadata_payload_sync (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    payload_key TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error TEXT,
    completed_at TIMESTAMPTZ,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_document_metadata_payload_sync_key
        CHECK (btrim(payload_key) <> ''),
    CONSTRAINT chk_document_metadata_payload_sync_attempts
        CHECK (attempts >= 0),
    CONSTRAINT chk_document_metadata_payload_sync_lease
        CHECK ((lease_token IS NULL) = (lease_expires_at IS NULL))
);

-- One pending job per document and payload key; completed rows remain as the
-- durable convergence history.
CREATE UNIQUE INDEX IF NOT EXISTS uq_document_metadata_payload_sync_pending
    ON context69.document_metadata_payload_sync (document_id, payload_key)
    WHERE completed_at IS NULL;

-- Due-order claim scan. The worker also filters on the lease.
CREATE INDEX IF NOT EXISTS idx_document_metadata_payload_sync_due
    ON context69.document_metadata_payload_sync (next_attempt_at, id)
    WHERE completed_at IS NULL;

DO $$
DECLARE
    affected BIGINT;
    imported_by BIGINT;
    stripped BIGINT;
    unsafe_doc BIGINT;
    unsafe_reason TEXT;
    imported_count BIGINT;
    enqueued BIGINT;
    removed_indexes BIGINT;
BEGIN
    SELECT count(*) INTO affected
    FROM context69.documents d
    WHERE d.metadata_json ? 'extraction';

    IF affected > 0 THEN
        -- Every affected document must have a trustworthy canonical snapshot.
        -- The stripped object must be exactly the immutable snapshot already
        -- recorded for the current hash, so stripping cannot change the hash.
        SELECT d.id, 'canonical snapshot does not match the stripped source metadata'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        WHERE d.metadata_json ? 'extraction'
          AND NOT EXISTS (
              SELECT 1
              FROM context69.document_versions v
              WHERE v.document_id = d.id
                AND v.record_hash = d.record_hash
                AND v.metadata_json IS NOT DISTINCT FROM (d.metadata_json - 'extraction')
                AND v.title IS NOT DISTINCT FROM d.title
                AND v.summary IS NOT DISTINCT FROM d.summary
                AND v.source_uri IS NOT DISTINCT FROM d.source_uri
          )
        ORDER BY d.id
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        SELECT d.id, 'canonical snapshot body is blank'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        JOIN context69.document_versions v
          ON v.document_id = d.id AND v.record_hash = d.record_hash
        WHERE d.metadata_json ? 'extraction'
          AND btrim(v.body_text) = ''
        ORDER BY d.id
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        -- Chunks must be present and all belong to the current source generation.
        SELECT d.id, 'chunk rows are missing or drifted from the current source hash'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        WHERE d.metadata_json ? 'extraction'
          AND (
              NOT EXISTS (SELECT 1 FROM context69.document_chunks c WHERE c.document_id = d.id)
              OR EXISTS (
                  SELECT 1 FROM context69.document_chunks c
                  WHERE c.document_id = d.id AND c.record_hash <> d.record_hash
              )
          )
        ORDER BY d.id
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        -- The retired namespace itself and every payload must be well-formed and
        -- within the documented result envelope.
        SELECT d.id, 'extraction metadata is not an object'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        WHERE d.metadata_json ? 'extraction'
          AND jsonb_typeof(d.metadata_json -> 'extraction') <> 'object'
        ORDER BY d.id
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        SELECT d.id, 'unknown enrichment schema key ' || entry.key
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        CROSS JOIN LATERAL jsonb_each(d.metadata_json -> 'extraction') AS entry
        WHERE d.metadata_json ? 'extraction'
          AND entry.key NOT IN ('stock.news.v1', 'stock.disclosure.v1')
        ORDER BY d.id, entry.key
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        SELECT d.id, 'enrichment payload is not a JSON object'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        CROSS JOIN LATERAL jsonb_each(d.metadata_json -> 'extraction') AS entry
        WHERE d.metadata_json ? 'extraction'
          AND jsonb_typeof(entry.value) <> 'object'
        ORDER BY d.id, entry.key
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        SELECT d.id, 'enrichment payload exceeds the result envelope bound'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        CROSS JOIN LATERAL jsonb_each(d.metadata_json -> 'extraction') AS entry
        WHERE d.metadata_json ? 'extraction'
          AND octet_length(entry.value::text) + 4096 > 262144
        ORDER BY d.id, entry.key
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        -- A composed library metadata source would reintroduce the key on the
        -- next file/folder refresh, so it blocks instead of being silently
        -- cleaned.
        SELECT d.id, 'library file or section metadata still carries the retired key'
          INTO unsafe_doc, unsafe_reason
        FROM context69.documents d
        JOIN context69.library_file_documents mapping ON mapping.document_id = d.id
        JOIN context69.library_files file ON file.id = mapping.file_id
        WHERE d.metadata_json ? 'extraction'
          AND (file.metadata_json ? 'extraction' OR mapping.section_metadata_json ? 'extraction')
        ORDER BY d.id
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        -- The rollout runs under a maintenance window; refuse to migrate while
        -- any ingestion work is active rather than racing a concurrent writer.
        IF EXISTS (
            SELECT 1
            FROM context69.task_items item
            JOIN context69.tasks task ON task.id = item.task_id
            WHERE item.status IN ('queued', 'running', 'waiting')
              AND task.status IN ('queued', 'running', 'waiting')
        ) THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: active ingestion tasks';
        END IF;
        IF EXISTS (
            SELECT 1
            FROM context69.task_docling_remote_jobs job
            WHERE job.status IN ('pending', 'running')
        ) THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: active docling remote jobs';
        END IF;

        -- Import ownership: the lowest-id existing enabled admin, never a hidden
        -- or hardcoded account. The import is deployment-owned data migration
        -- and is never presented as the original author/model.
        SELECT min(u.id) INTO imported_by
        FROM context69.users u
        WHERE u.is_admin
          AND u.disabled_at IS NULL;
        IF imported_by IS NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: affected data exists but no active admin can own the imported records';
        END IF;

        -- Capture the exact affected set before any mutation.
        CREATE TEMP TABLE _ctx69_retire_docs ON COMMIT DROP AS
        SELECT d.id AS document_id, d.record_hash
        FROM context69.documents d
        WHERE d.metadata_json ? 'extraction';

        CREATE TEMP TABLE _ctx69_retire_imports ON COMMIT DROP AS
        SELECT d.id AS document_id,
               entry.key AS schema_ref,
               d.record_hash AS source_record_hash,
               entry.value AS result,
               'sqlx-retire:' || encode(
                   sha256(convert_to(
                       jsonb_build_array(d.id, entry.key, d.record_hash, entry.value)::text,
                       'UTF8'
                   )),
                   'hex'
               ) AS idempotency_key
        FROM context69.documents d
        CROSS JOIN LATERAL jsonb_each(d.metadata_json -> 'extraction') AS entry
        WHERE d.metadata_json ? 'extraction';

        -- Replays must be byte-exact. A changed envelope under an existing
        -- identity aborts instead of being masked by ON CONFLICT DO NOTHING.
        SELECT e.document_id, 'existing imported result has a different envelope'
          INTO unsafe_doc, unsafe_reason
        FROM _ctx69_retire_imports e
        JOIN context69.document_results r
          ON r.document_id = e.document_id
         AND r.author_user_id = imported_by
         AND r.producer = 'context69.enrichment-import'
         AND r.idempotency_key = e.idempotency_key
        WHERE r.result_kind IS DISTINCT FROM 'imported-enrichment'
           OR r.schema_ref IS DISTINCT FROM e.schema_ref
           OR r.source_record_hash IS DISTINCT FROM e.source_record_hash
           OR r.result IS DISTINCT FROM e.result
           OR r.provenance IS DISTINCT FROM jsonb_build_object(
                  'workflow_version', 'context69-enrichment-migration-20261017080707',
                  'run_id', 'context69-enrichment-retirement-sqlx'
              )
        ORDER BY e.document_id, e.schema_ref
        LIMIT 1;
        IF unsafe_doc IS NOT NULL THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: document % %', unsafe_doc, unsafe_reason;
        END IF;

        INSERT INTO context69.document_results (
            document_id,
            author_user_id,
            producer,
            result_kind,
            schema_ref,
            source_record_hash,
            idempotency_key,
            result,
            provenance
        )
        SELECT e.document_id,
               imported_by,
               'context69.enrichment-import',
               'imported-enrichment',
               e.schema_ref,
               e.source_record_hash,
               e.idempotency_key,
               e.result,
               jsonb_build_object(
                   'workflow_version', 'context69-enrichment-migration-20261017080707',
                   'run_id', 'context69-enrichment-retirement-sqlx'
               )
        FROM _ctx69_retire_imports e
        ON CONFLICT (document_id, author_user_id, producer, idempotency_key) DO NOTHING;

        -- Every expected import must now be present with the exact envelope.
        SELECT count(*) INTO imported_count
        FROM _ctx69_retire_imports e
        JOIN context69.document_results r
          ON r.document_id = e.document_id
         AND r.author_user_id = imported_by
         AND r.producer = 'context69.enrichment-import'
         AND r.idempotency_key = e.idempotency_key
         AND r.result_kind = 'imported-enrichment'
         AND r.schema_ref = e.schema_ref
         AND r.source_record_hash = e.source_record_hash
         AND r.result = e.result;
        IF imported_count <> (SELECT count(*) FROM _ctx69_retire_imports) THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: imported result count mismatch (% of %)',
                imported_count, (SELECT count(*) FROM _ctx69_retire_imports);
        END IF;

        -- Strip only the retired key, and only from the exact captured set.
        UPDATE context69.documents d
        SET metadata_json = d.metadata_json - 'extraction',
            updated_at = now()
        FROM _ctx69_retire_docs t
        WHERE d.id = t.document_id
          AND d.record_hash = t.record_hash
          AND d.metadata_json ? 'extraction';
        GET DIAGNOSTICS stripped = ROW_COUNT;
        IF stripped <> affected THEN
            RAISE EXCEPTION 'context69 enrichment retirement blocked: stripped % of % documents', stripped, affected;
        END IF;

        -- Durable mirror convergence: one job per affected document and key. The
        -- worker retries until the external payload no longer carries the key.
        INSERT INTO context69.document_metadata_payload_sync (document_id, payload_key)
        SELECT t.document_id, 'metadata_index.extraction'
        FROM _ctx69_retire_docs t
        ON CONFLICT (document_id, payload_key) WHERE completed_at IS NULL DO NOTHING;
        GET DIAGNOSTICS enqueued = ROW_COUNT;
    ELSE
        enqueued := 0;
    END IF;

    -- Independent convergence that does not depend on any document still
    -- carrying the key, so a later rollout also retires it from history.
    -- Clear only the obsolete label; status, counters, errors, attempts,
    -- sibling items, and every other label stay untouched.
    UPDATE context69.tasks SET failure_stage = NULL WHERE failure_stage = 'extraction';
    UPDATE context69.task_items SET failure_stage = NULL WHERE failure_stage = 'extraction';

    -- Remove only extraction/extraction.* metadata-index definitions and their
    -- owned values (the values table cascades from the definition).
    DELETE FROM context69.metadata_index_definitions
    WHERE field_path = 'extraction' OR field_path LIKE 'extraction.%';
    GET DIAGNOSTICS removed_indexes = ROW_COUNT;

    -- Cached search results that still expose a removed key or index must miss
    -- once.
    IF affected > 0 OR removed_indexes > 0 THEN
        INSERT INTO context69.search_generations (scope, generation, updated_at)
        VALUES ('global', 1, now())
        ON CONFLICT (scope) DO UPDATE
        SET generation = context69.search_generations.generation + 1,
            updated_at = now();
    END IF;

    RAISE NOTICE 'context69 enrichment retirement: documents=%, imports=%, enqueued=%, removed_indexes=%',
        affected, COALESCE(imported_count, 0), enqueued, removed_indexes;
END $$;
