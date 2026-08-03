-- Apply during the maintenance window after the database backup and after all
-- legacy and unified workers have stopped. Legacy rows are copied to fresh
-- task/item ids before the legacy tables are removed.

DO $$
DECLARE
    docling_state TEXT;
    docling_probe_token UUID;
BEGIN
    SELECT state, probe_lease_token
    INTO docling_state, docling_probe_token
    FROM context69.library_dependency_gates
    WHERE dependency_key = 'docling';

    IF NOT FOUND THEN
        RAISE EXCEPTION 'docling dependency gate is missing';
    END IF;
    IF docling_state <> 'closed' OR docling_probe_token IS NOT NULL THEN
        RAISE EXCEPTION
            'docling dependency gate must be closed with no probe lease before unified task migration (state %, probe lease %)',
            docling_state,
            docling_probe_token;
    END IF;
END
$$;

ALTER TABLE context69.tasks
    ALTER COLUMN user_id DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS waiting_count BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS stage TEXT,
    ADD COLUMN IF NOT EXISTS waiting_reason TEXT,
    ADD COLUMN IF NOT EXISTS dependency_key TEXT,
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

ALTER TABLE context69.task_items
    ADD COLUMN IF NOT EXISTS stage TEXT,
    ADD COLUMN IF NOT EXISTS waiting_reason TEXT,
    ADD COLUMN IF NOT EXISTS dependency_key TEXT,
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS file_id UUID REFERENCES context69.library_files(id) ON DELETE SET NULL;

-- Existing unified work is adopted by the new worker. Clearing leases is safe
-- only because the deployment procedure stops every old worker first.
UPDATE context69.tasks
SET lease_token = NULL,
    lease_until = NULL,
    updated_at = now()
WHERE status IN ('queued', 'running', 'waiting');

-- No old worker may leave an attempt marked active after the maintenance
-- window. The new worker records a fresh attempt after it claims the item.
UPDATE context69.task_attempts
SET status = 'interrupted',
    failure_stage = 'lease',
    error_message = 'attempt interrupted during unified task migration',
    finished_at = now()
WHERE finished_at IS NULL;

-- Preserve the group permission scope for unified tasks created before the
-- group id became the authoritative ownership field.
UPDATE context69.tasks task
SET group_id = group_scope.id,
    group_path = COALESCE(task.group_path, group_scope.full_path),
    updated_at = now()
FROM context69.groups group_scope
WHERE task.group_id IS NULL
  AND task.group_path IS NOT NULL
  AND group_scope.full_path = task.group_path;

UPDATE context69.task_items
SET status = CASE WHEN status = 'running' THEN 'queued' ELSE status END,
    waiting_reason = CASE WHEN status = 'waiting' THEN waiting_reason ELSE NULL END,
    dependency_key = CASE WHEN status = 'waiting' THEN dependency_key ELSE NULL END,
    lease_token = NULL,
    lease_until = NULL,
    next_attempt_at = CASE
        WHEN status IN ('queued', 'running') THEN COALESCE(next_attempt_at, now())
        ELSE next_attempt_at
    END,
    updated_at = now()
WHERE status IN ('queued', 'running', 'waiting');

-- Older unified items may have carried a file id only in their payload. Reuse
-- that reference only when it is a valid, existing file; malformed payloads
-- remain visible to the worker instead of causing a migration cast failure.
UPDATE context69.task_items item
SET file_id = file.id,
    resource_id = COALESCE(item.resource_id, file.id::TEXT),
    updated_at = now()
FROM context69.library_files file
WHERE item.file_id IS NULL
  AND item.payload ->> 'file_id' ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
  AND file.id::TEXT = item.payload ->> 'file_id';

UPDATE context69.task_items item
SET stage = COALESCE(
        item.stage,
        CASE task.kind
            WHEN 'url_batch' THEN 'download'
            WHEN 'file_batch' THEN 'storage'
            WHEN 'text_batch' THEN 'storage'
            WHEN 'source_sync' THEN 'sync'
            WHEN 'delete_batch' THEN 'delete'
            WHEN 'translation' THEN 'translation'
            WHEN 'vector_rebuild' THEN 'indexing'
            ELSE 'finalize'
        END
    )
FROM context69.tasks task
WHERE task.id = item.task_id
  AND item.stage IS NULL;

CREATE TEMP TABLE unified_legacy_task_map (
    legacy_kind TEXT NOT NULL,
    legacy_id UUID NOT NULL,
    task_id UUID NOT NULL DEFAULT gen_random_uuid(),
    item_id UUID NOT NULL DEFAULT gen_random_uuid(),
    existing_task_id UUID,
    existing_item_id UUID,
    group_id BIGINT NOT NULL,
    group_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload JSONB NOT NULL,
    file_id UUID,
    stage TEXT NOT NULL,
    item_status TEXT NOT NULL,
    failure_stage TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (legacy_kind, legacy_id)
) ON COMMIT DROP;

-- Build one row per active URL import. A URL import can reuse one existing
-- active unified item only when the match is unambiguous. Every other case is
-- migrated to a fresh task/item, with an explicit migration failure when the
-- old relationship cannot be trusted.
INSERT INTO unified_legacy_task_map (
    legacy_kind,
    legacy_id,
    existing_task_id,
    existing_item_id,
    group_id,
    group_path,
    kind,
    payload,
    file_id,
    stage,
    item_status,
    failure_stage,
    error_message,
    created_at
)
SELECT
    'url',
    job.id,
    CASE WHEN failure.message IS NULL
              AND candidate.candidate_count = 1
              AND candidate.candidate_file_id IS NOT DISTINCT FROM job.file_id
         THEN candidate.task_id END,
    CASE WHEN failure.message IS NULL
              AND candidate.candidate_count = 1
              AND candidate.candidate_file_id IS NOT DISTINCT FROM job.file_id
         THEN candidate.item_id END,
    job.group_id,
    group_scope.full_path,
    'url_batch',
    jsonb_build_object(
        'url', job.source_url,
        'folder_id', job.folder_id,
        'filename', job.requested_filename,
        'media_type', job.requested_media_type,
        'metadata', CASE WHEN job.metadata_provided THEN jsonb_build_object(
            'external_id', job.external_id,
            'source_uri', job.source_uri,
            'published_at', job.published_at,
            'metadata_json', job.metadata_json
        ) ELSE NULL END,
        'translation', CASE WHEN job.translation_provided THEN jsonb_build_object(
            'source_locale', job.translation_source_locale,
            'target_locales', job.translation_target_locales
        ) ELSE NULL END,
    'section_payload', ingest.section_payload
    ),
    CASE WHEN file.id IS NOT NULL THEN job.file_id ELSE NULL END,
    CASE
        WHEN job.status = 'ingesting' AND ingest.requires_docling THEN 'docling'
        WHEN job.status = 'ingesting' THEN 'embedding'
        ELSE 'download'
    END,
    CASE WHEN failure.message IS NULL THEN 'queued' ELSE 'failed' END,
    CASE WHEN failure.message IS NULL THEN NULL ELSE 'migration' END,
    failure.message,
    job.created_at
FROM context69.library_url_import_jobs job
JOIN context69.groups group_scope ON group_scope.id = job.group_id
LEFT JOIN context69.library_files file ON file.id = job.file_id
LEFT JOIN context69.library_ingest_jobs ingest ON ingest.id = job.ingest_job_id
LEFT JOIN LATERAL (
    SELECT
        (array_agg(task.id ORDER BY task.id))[1] AS task_id,
        (array_agg(item.id ORDER BY item.id))[1] AS item_id,
        (array_agg(item.file_id ORDER BY item.id))[1] AS candidate_file_id,
        count(*)::BIGINT AS candidate_count
    FROM context69.task_items item
    JOIN context69.tasks task ON task.id = item.task_id
    WHERE task.kind = 'url_batch'
      AND task.group_id = job.group_id
      AND item.payload ->> 'url' = job.source_url
      AND item.status IN ('queued', 'running', 'waiting')
) candidate ON TRUE
LEFT JOIN LATERAL (
    SELECT message
    FROM (
        VALUES
            (CASE WHEN candidate.candidate_count > 1
                  THEN 'multiple active unified task items match this URL import'
                  END),
            (CASE WHEN candidate.candidate_count = 1
                        AND candidate.candidate_file_id IS DISTINCT FROM job.file_id
                  THEN 'active URL import and unified task item reference different files'
                  END),
            (CASE WHEN (SELECT count(*)
                         FROM context69.library_url_import_jobs duplicate
                         WHERE duplicate.group_id = job.group_id
                           AND duplicate.dedupe_key = job.dedupe_key
                           AND duplicate.status IN ('queued', 'downloading', 'ingesting')) > 1
                  THEN 'multiple active URL imports have the same dedupe key'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND job.file_id IS NULL
                  THEN 'active URL import is missing its file'
                  END),
            (CASE WHEN job.file_id IS NOT NULL AND file.id IS NULL
                  THEN 'active URL import references a missing file'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND job.ingest_job_id IS NULL
                  THEN 'active URL import is missing its ingest child job'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND job.ingest_job_id IS NOT NULL
                        AND ingest.id IS NULL
                  THEN 'active URL import references a missing ingest child job'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND ingest.status NOT IN ('pending', 'running')
                  THEN 'active URL import references a non-active ingest child job'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND ingest.id IS NOT NULL
                        AND ingest.file_id IS DISTINCT FROM job.file_id
                  THEN 'active URL import and ingest child reference different files'
                  END),
            (CASE WHEN job.file_id IS NOT NULL
                        AND (SELECT count(*)
                             FROM context69.library_ingest_jobs duplicate
                             WHERE duplicate.file_id = job.file_id
                               AND duplicate.status IN ('pending', 'running')) > 1
                  THEN 'multiple active ingest jobs reference the same file'
                  END),
            (CASE WHEN job.file_id IS NOT NULL
                        AND (SELECT count(*)
                             FROM context69.library_url_import_jobs duplicate
                             WHERE duplicate.file_id = job.file_id
                               AND duplicate.status IN ('queued', 'downloading', 'ingesting')) > 1
                  THEN 'multiple active URL imports reference the same file'
                  END),
            (CASE WHEN job.file_id IS NOT NULL
                        AND EXISTS (
                            SELECT 1
                            FROM context69.library_ingest_jobs duplicate
                            WHERE duplicate.file_id = job.file_id
                              AND duplicate.id IS DISTINCT FROM job.ingest_job_id
                              AND duplicate.status IN ('pending', 'running')
                        )
                  THEN 'active URL import shares its file with another active ingest job'
                  END),
            (CASE WHEN job.status = 'ingesting'
                        AND ingest.id IS NOT NULL
                        AND (SELECT count(*)
                             FROM context69.library_url_import_jobs duplicate
                             WHERE duplicate.ingest_job_id = job.ingest_job_id
                               AND duplicate.status IN ('queued', 'downloading', 'ingesting')) > 1
                  THEN 'active ingest child job is referenced by multiple URL imports'
                  END)
    ) errors(message)
    WHERE message IS NOT NULL
    LIMIT 1
) failure ON TRUE
WHERE job.status IN ('queued', 'downloading', 'ingesting');

-- Reattach the unambiguous matches without changing their task ownership.
UPDATE context69.task_items item
SET file_id = map.file_id,
    resource_id = map.file_id::TEXT,
    stage = map.stage,
    status = map.item_status,
    failure_stage = map.failure_stage,
    error_message = map.error_message,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = CASE WHEN map.item_status = 'queued' THEN now() ELSE NULL END,
    lease_token = NULL,
    lease_until = NULL,
    finished_at = CASE WHEN map.item_status = 'failed' THEN now() ELSE NULL END,
    updated_at = now()
FROM unified_legacy_task_map map
WHERE map.legacy_kind = 'url'
  AND map.existing_item_id = item.id;

-- Never let several legacy rows silently reuse one active unified item. The
-- existing item is made terminal and every legacy row gets its own explicit
-- migration failure item below.
UPDATE context69.task_items item
SET status = 'failed',
    failure_stage = 'migration',
    error_message = 'multiple active legacy URL imports reference one unified task item',
    retryable = FALSE,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = NULL,
    lease_token = NULL,
    lease_until = NULL,
    finished_at = now(),
    updated_at = now()
WHERE item.id IN (
    SELECT existing_item_id
    FROM unified_legacy_task_map
    WHERE legacy_kind = 'url'
      AND existing_item_id IS NOT NULL
    GROUP BY existing_item_id
    HAVING count(*) > 1
);

UPDATE unified_legacy_task_map map
SET existing_task_id = NULL,
    existing_item_id = NULL,
    item_status = 'failed',
    failure_stage = 'migration',
    error_message = 'multiple active legacy URL imports reference one unified task item'
WHERE map.legacy_kind = 'url'
  AND map.existing_item_id IN (
      SELECT existing_item_id
      FROM unified_legacy_task_map
      WHERE legacy_kind = 'url'
        AND existing_item_id IS NOT NULL
      GROUP BY existing_item_id
      HAVING count(*) > 1
  );

-- All other URL imports get fresh task/item ids. There is intentionally no
-- conflict-ignore clause: fresh UUIDs and the temporary primary key make every
-- old active row auditable.
INSERT INTO context69.tasks (
    id, user_id, group_id, kind, status, group_path, total_count,
    queued_count, failed_count, stage, failure_stage, error_summary,
    next_attempt_at, created_at, updated_at
)
SELECT
    map.task_id,
    NULL,
    map.group_id,
    map.kind,
    CASE WHEN map.item_status = 'failed' THEN 'failed' ELSE 'queued' END,
    map.group_path,
    1,
    CASE WHEN map.item_status = 'queued' THEN 1 ELSE 0 END,
    CASE WHEN map.item_status = 'failed' THEN 1 ELSE 0 END,
    map.stage,
    map.failure_stage,
    map.error_message,
    CASE WHEN map.item_status = 'queued' THEN now() ELSE NULL END,
    map.created_at,
    now()
FROM unified_legacy_task_map map
WHERE map.legacy_kind = 'url'
  AND map.existing_item_id IS NULL;

INSERT INTO context69.task_items (
    id, task_id, ordinal, payload, status, resource_id, file_id, stage,
    failure_stage, error_message, retryable, next_attempt_at, finished_at,
    created_at, updated_at
)
SELECT
    map.item_id,
    map.task_id,
    0,
    map.payload,
    map.item_status,
    map.file_id::TEXT,
    map.file_id,
    map.stage,
    map.failure_stage,
    map.error_message,
    map.item_status <> 'failed',
    CASE WHEN map.item_status = 'queued' THEN now() ELSE NULL END,
    CASE WHEN map.item_status = 'failed' THEN now() ELSE NULL END,
    map.created_at,
    now()
FROM unified_legacy_task_map map
WHERE map.legacy_kind = 'url'
  AND map.existing_item_id IS NULL;

-- Active standalone ingest jobs become file tasks. An ingest child referenced
-- by an active URL import is owned by that URL item and is not duplicated.
INSERT INTO unified_legacy_task_map (
    legacy_kind,
    legacy_id,
    group_id,
    group_path,
    kind,
    payload,
    file_id,
    stage,
    item_status,
    failure_stage,
    error_message,
    created_at
)
SELECT
    'ingest',
    job.id,
    job.group_id,
    group_scope.full_path,
    'file_batch',
    jsonb_build_object(
        'file_id', job.file_id,
        'section_payload', job.section_payload
    ),
    CASE WHEN file.id IS NOT NULL THEN job.file_id ELSE NULL END,
    CASE WHEN job.requires_docling THEN 'docling' ELSE 'embedding' END,
    CASE WHEN failure.message IS NULL AND file.id IS NOT NULL THEN 'queued' ELSE 'failed' END,
    CASE WHEN failure.message IS NULL AND file.id IS NOT NULL THEN NULL ELSE 'migration' END,
    CASE WHEN file.id IS NULL THEN 'active ingest job references a missing file'
         ELSE failure.message END,
    job.created_at
FROM context69.library_ingest_jobs job
JOIN context69.groups group_scope ON group_scope.id = job.group_id
LEFT JOIN context69.library_files file ON file.id = job.file_id
LEFT JOIN LATERAL (
    SELECT message
    FROM (
        VALUES
            (CASE WHEN (SELECT count(*)
                        FROM context69.library_ingest_jobs duplicate
                        WHERE duplicate.file_id = job.file_id
                          AND duplicate.status IN ('pending', 'running')) > 1
                  THEN 'multiple active ingest jobs reference the same file'
                  END),
            (CASE WHEN EXISTS (
                        SELECT 1
                        FROM context69.library_url_import_jobs url_job
                        WHERE url_job.file_id = job.file_id
                          AND url_job.status IN ('queued', 'downloading', 'ingesting')
                    )
                  THEN 'active ingest job shares its file with an active URL import'
                  END)
    ) errors(message)
    WHERE message IS NOT NULL
    LIMIT 1
) failure ON TRUE
WHERE job.status IN ('pending', 'running')
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_url_import_jobs url_job
      WHERE url_job.ingest_job_id = job.id
        AND url_job.status IN ('queued', 'downloading', 'ingesting')
  );

INSERT INTO context69.tasks (
    id, user_id, group_id, kind, status, group_path, total_count,
    queued_count, failed_count, stage, failure_stage, error_summary,
    next_attempt_at, created_at, updated_at
)
SELECT
    map.task_id,
    NULL,
    map.group_id,
    map.kind,
    CASE WHEN map.item_status = 'failed' THEN 'failed' ELSE 'queued' END,
    map.group_path,
    1,
    CASE WHEN map.item_status = 'queued' THEN 1 ELSE 0 END,
    CASE WHEN map.item_status = 'failed' THEN 1 ELSE 0 END,
    map.stage,
    map.failure_stage,
    map.error_message,
    CASE WHEN map.item_status = 'queued' THEN now() ELSE NULL END,
    map.created_at,
    now()
FROM unified_legacy_task_map map
WHERE map.legacy_kind = 'ingest';

INSERT INTO context69.task_items (
    id, task_id, ordinal, payload, status, resource_id, file_id, stage,
    failure_stage, error_message, retryable, next_attempt_at, finished_at,
    created_at, updated_at
)
SELECT
    map.item_id,
    map.task_id,
    0,
    map.payload,
    map.item_status,
    map.file_id::TEXT,
    map.file_id,
    map.stage,
    map.failure_stage,
    map.error_message,
    map.item_status <> 'failed',
    CASE WHEN map.item_status = 'queued' THEN now() ELSE NULL END,
    CASE WHEN map.item_status = 'failed' THEN now() ELSE NULL END,
    map.created_at,
    now()
FROM unified_legacy_task_map map
WHERE map.legacy_kind = 'ingest';

-- Recompute every touched task from its items, including existing tasks that
-- adopted a legacy URL row.
UPDATE context69.tasks task
SET queued_count = counts.queued_count,
    running_count = counts.running_count,
    waiting_count = counts.waiting_count,
    succeeded_count = counts.succeeded_count,
    failed_count = counts.failed_count,
    cancelled_count = counts.cancelled_count,
    status = CASE
        WHEN task.status = 'cancelled' THEN 'cancelled'
        WHEN counts.cancelled_count = task.total_count THEN 'cancelled'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = task.total_count
             AND counts.failed_count = 0 THEN 'succeeded'
        WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = task.total_count THEN 'failed'
        WHEN counts.running_count > 0 THEN 'running'
        WHEN counts.waiting_count > 0 AND counts.queued_count = 0 THEN 'waiting'
        ELSE 'queued'
    END,
    failure_stage = COALESCE(
        (SELECT item.failure_stage FROM context69.task_items item
         WHERE item.task_id = task.id AND item.status = 'failed'
         ORDER BY item.ordinal LIMIT 1),
        task.failure_stage
    ),
    error_summary = COALESCE(
        (SELECT item.error_message FROM context69.task_items item
         WHERE item.task_id = task.id AND item.status = 'failed'
         ORDER BY item.ordinal LIMIT 1),
        task.error_summary
    ),
    stage = current_item.stage,
    waiting_reason = current_item.waiting_reason,
    dependency_key = current_item.dependency_key,
    next_attempt_at = current_item.next_attempt_at,
    updated_at = now()
FROM (
    SELECT task_id,
        count(*) FILTER (WHERE status = 'queued')::BIGINT AS queued_count,
        count(*) FILTER (WHERE status = 'running')::BIGINT AS running_count,
        count(*) FILTER (WHERE status = 'waiting')::BIGINT AS waiting_count,
        count(*) FILTER (WHERE status = 'succeeded')::BIGINT AS succeeded_count,
        count(*) FILTER (WHERE status = 'failed')::BIGINT AS failed_count,
        count(*) FILTER (WHERE status = 'cancelled')::BIGINT AS cancelled_count
    FROM context69.task_items
    GROUP BY task_id
) counts
LEFT JOIN LATERAL (
    SELECT item.stage, item.waiting_reason, item.dependency_key, item.next_attempt_at
    FROM context69.task_items item
    WHERE item.task_id = counts.task_id
      AND item.status IN ('queued', 'running', 'waiting')
    ORDER BY
        CASE item.status
            WHEN 'queued' THEN 0
            WHEN 'running' THEN 1
            ELSE 2
        END,
        item.next_attempt_at NULLS FIRST,
        item.ordinal
    LIMIT 1
) current_item ON TRUE
WHERE task.id = counts.task_id;

-- Keep library_files consistent with the new active item state before the old
-- job tables disappear. Terminal file states are otherwise preserved.
UPDATE context69.library_files file
SET ingest_status = CASE
        WHEN state.has_failed THEN 'failed'
        ELSE 'pending'
    END,
    error_message = CASE WHEN state.has_failed THEN state.error_message ELSE NULL END,
    ingested_at = CASE WHEN state.has_failed THEN NULL ELSE file.ingested_at END,
    updated_at = now()
FROM (
    SELECT
        item.file_id,
        bool_or(item.status = 'failed') AS has_failed,
        bool_or(
            item.status IN ('queued', 'running', 'waiting')
            AND item.stage IN ('download', 'storage', 'docling', 'embedding', 'indexing')
        ) AS has_active_ingest,
        min(item.error_message) FILTER (WHERE item.status = 'failed') AS error_message
    FROM context69.task_items item
    JOIN context69.tasks task ON task.id = item.task_id
    WHERE item.file_id IS NOT NULL
      AND task.kind IN ('url_batch', 'file_batch', 'text_batch')
      AND (
          item.status IN ('queued', 'running', 'waiting')
          AND item.stage IN ('download', 'storage', 'docling', 'embedding', 'indexing')
          OR item.status = 'failed'
             AND item.failure_stage IN (
                 'download', 'storage', 'docling', 'embedding', 'indexing', 'migration'
             )
      )
    GROUP BY item.file_id
) state
WHERE state.file_id = file.id;

CREATE INDEX IF NOT EXISTS idx_task_items_stage_status
    ON context69.task_items (stage, status, next_attempt_at, created_at);
CREATE INDEX IF NOT EXISTS idx_task_items_dependency
    ON context69.task_items (dependency_key, status, next_attempt_at);
CREATE INDEX IF NOT EXISTS idx_tasks_group_created_at
    ON context69.tasks (group_id, created_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS uq_tasks_active_vector_rebuild
    ON context69.tasks (kind)
    WHERE kind = 'vector_rebuild'
      AND status IN ('queued', 'running', 'waiting');

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'context69.tasks'::regclass
          AND conname = 'tasks_status_valid'
    ) THEN
        ALTER TABLE context69.tasks
            ADD CONSTRAINT tasks_status_valid
            CHECK (status IN ('queued', 'running', 'waiting', 'succeeded', 'failed', 'cancelled'));
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conrelid = 'context69.task_items'::regclass
          AND conname = 'task_items_status_valid'
    ) THEN
        ALTER TABLE context69.task_items
            ADD CONSTRAINT task_items_status_valid
            CHECK (status IN ('queued', 'running', 'waiting', 'succeeded', 'failed', 'cancelled'));
    END IF;
END
$$;

DO $$
DECLARE
    active_url_count BIGINT;
    mapped_url_count BIGINT;
    active_standalone_ingest_count BIGINT;
    mapped_ingest_count BIGINT;
    unaccounted_ingest_count BIGINT;
    unmaterialized_item_count BIGINT;
    orphan_file_item_count BIGINT;
    task_count_mismatch BIGINT;
BEGIN
    SELECT count(*)
    INTO active_url_count
    FROM context69.library_url_import_jobs
    WHERE status IN ('queued', 'downloading', 'ingesting');

    SELECT count(*)
    INTO mapped_url_count
    FROM unified_legacy_task_map
    WHERE legacy_kind = 'url';

    IF active_url_count <> mapped_url_count THEN
        RAISE EXCEPTION
            'active URL migration is incomplete: expected %, mapped %',
            active_url_count,
            mapped_url_count;
    END IF;

    SELECT count(*)
    INTO active_standalone_ingest_count
    FROM context69.library_ingest_jobs ingest
    WHERE ingest.status IN ('pending', 'running')
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = ingest.id
            AND url_job.status IN ('queued', 'downloading', 'ingesting')
      );

    SELECT count(*)
    INTO mapped_ingest_count
    FROM unified_legacy_task_map
    WHERE legacy_kind = 'ingest';

    IF active_standalone_ingest_count <> mapped_ingest_count THEN
        RAISE EXCEPTION
            'standalone ingest migration is incomplete: expected %, mapped %',
            active_standalone_ingest_count,
            mapped_ingest_count;
    END IF;

    SELECT count(*)
    INTO unaccounted_ingest_count
    FROM context69.library_ingest_jobs ingest
    WHERE ingest.status IN ('pending', 'running')
      AND NOT EXISTS (
          SELECT 1
          FROM unified_legacy_task_map map
          WHERE map.legacy_kind = 'ingest'
            AND map.legacy_id = ingest.id
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = ingest.id
            AND url_job.status IN ('queued', 'downloading', 'ingesting')
      );

    IF unaccounted_ingest_count <> 0 THEN
        RAISE EXCEPTION
            'active ingest rows are neither mapped nor owned by an active URL: %',
            unaccounted_ingest_count;
    END IF;

    SELECT count(*)
    INTO unmaterialized_item_count
    FROM unified_legacy_task_map map
    WHERE NOT EXISTS (
        SELECT 1
        FROM context69.task_items item
        WHERE item.id = COALESCE(map.existing_item_id, map.item_id)
    );

    IF unmaterialized_item_count <> 0 THEN
        RAISE EXCEPTION
            'legacy task map contains unmaterialized items: %',
            unmaterialized_item_count;
    END IF;

    SELECT count(*)
    INTO orphan_file_item_count
    FROM context69.task_items item
    WHERE item.file_id IS NOT NULL
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_files file
          WHERE file.id = item.file_id
      );

    IF orphan_file_item_count <> 0 THEN
        RAISE EXCEPTION
            'unified task items reference missing files: %',
            orphan_file_item_count;
    END IF;

    SELECT count(*)
    INTO task_count_mismatch
    FROM context69.tasks task
    WHERE task.total_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id
          )
       OR task.queued_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'queued'
          )
       OR task.running_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'running'
          )
       OR task.waiting_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'waiting'
          )
       OR task.succeeded_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'succeeded'
          )
       OR task.failed_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'failed'
          )
       OR task.cancelled_count <> (
              SELECT count(*)
              FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'cancelled'
          );

    IF task_count_mismatch <> 0 THEN
        RAISE EXCEPTION
            'unified task counters do not match task items: % tasks',
            task_count_mismatch;
    END IF;
END
$$;

-- Legacy terminal history is intentionally not retained online. Active rows
-- have already been copied above and the backup is the rollback source.
DELETE FROM context69.library_url_import_jobs;
DELETE FROM context69.library_ingest_jobs;
DROP TABLE context69.library_url_import_jobs;
DROP TABLE context69.library_ingest_jobs;
