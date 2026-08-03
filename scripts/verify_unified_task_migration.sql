\set ON_ERROR_STOP on

-- Run this against a production snapshot before the maintenance migration.
-- The temporary result table is intentionally dropped with the transaction.
CREATE TEMP TABLE unified_task_migration_checks (
    check_name TEXT PRIMARY KEY,
    passed BOOLEAN NOT NULL,
    observed BIGINT NOT NULL,
    detail TEXT NOT NULL
) ON COMMIT DROP;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'docling_gate',
    COALESCE(gate.state = 'closed' AND gate.probe_lease_token IS NULL, FALSE),
    CASE WHEN gate.state = 'closed' AND gate.probe_lease_token IS NULL THEN 0 ELSE 1 END,
    COALESCE(
        format('state=%s, probe_lease=%s', gate.state, gate.probe_lease_token),
        'docling dependency gate is missing'
    )
FROM (SELECT 1 AS marker) marker
LEFT JOIN context69.library_dependency_gates gate
    ON gate.dependency_key = 'docling';

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'active_url_relationships',
    bad.count = 0,
    bad.count,
    'active URL rows with missing or inconsistent file/ingest references'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM context69.library_url_import_jobs job
    LEFT JOIN context69.library_files file ON file.id = job.file_id
    LEFT JOIN context69.library_ingest_jobs ingest ON ingest.id = job.ingest_job_id
    WHERE job.status IN ('queued', 'downloading', 'ingesting')
      AND (
          (job.file_id IS NOT NULL AND file.id IS NULL)
          OR (job.status = 'ingesting' AND job.file_id IS NULL)
          OR (job.status = 'ingesting' AND job.ingest_job_id IS NULL)
          OR (job.status = 'ingesting' AND ingest.id IS NULL)
          OR (job.status = 'ingesting' AND ingest.status NOT IN ('pending', 'running'))
          OR (job.status = 'ingesting' AND ingest.file_id IS DISTINCT FROM job.file_id)
      )
) bad;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'duplicate_active_ingest_files',
    bad.count = 0,
    bad.count,
    'files referenced by more than one active ingest job'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM (
        SELECT file_id
        FROM context69.library_ingest_jobs
        WHERE status IN ('pending', 'running')
        GROUP BY file_id
        HAVING count(*) > 1
    ) duplicate_files
) bad;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'duplicate_active_url_files',
    bad.count = 0,
    bad.count,
    'files referenced by more than one active URL import'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM (
        SELECT file_id
        FROM context69.library_url_import_jobs
        WHERE status IN ('queued', 'downloading', 'ingesting')
          AND file_id IS NOT NULL
        GROUP BY file_id
        HAVING count(*) > 1
    ) duplicate_files
) bad;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'duplicate_active_url_dedupe_keys',
    bad.count = 0,
    bad.count,
    'group/dedupe keys referenced by more than one active URL import'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM (
        SELECT group_id, dedupe_key
        FROM context69.library_url_import_jobs
        WHERE status IN ('queued', 'downloading', 'ingesting')
        GROUP BY group_id, dedupe_key
        HAVING count(*) > 1
    ) duplicate_keys
) bad;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'task_counter_consistency',
    bad.count = 0,
    bad.count,
    'unified task counters do not match task items'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM context69.tasks task
    WHERE task.total_count <> (
              SELECT count(*) FROM context69.task_items item WHERE item.task_id = task.id
          )
       OR task.queued_count <> (
              SELECT count(*) FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'queued'
          )
       OR task.running_count <> (
              SELECT count(*) FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'running'
          )
       OR task.succeeded_count <> (
              SELECT count(*) FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'succeeded'
          )
       OR task.failed_count <> (
              SELECT count(*) FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'failed'
          )
       OR task.cancelled_count <> (
              SELECT count(*) FROM context69.task_items item
              WHERE item.task_id = task.id AND item.status = 'cancelled'
          )
) bad;

INSERT INTO unified_task_migration_checks (check_name, passed, observed, detail)
SELECT
    'zero_item_tasks',
    bad.count = 0,
    bad.count,
    'tasks without a corresponding task item'
FROM (
    SELECT count(*)::BIGINT AS count
    FROM context69.tasks task
    WHERE NOT EXISTS (
        SELECT 1 FROM context69.task_items item WHERE item.task_id = task.id
    )
) bad;

SELECT check_name, passed, observed, detail
FROM unified_task_migration_checks
ORDER BY check_name;

DO $$
DECLARE
    failed_count BIGINT;
BEGIN
    SELECT count(*) INTO failed_count
    FROM unified_task_migration_checks
    WHERE NOT passed;
    IF failed_count > 0 THEN
        RAISE EXCEPTION 'unified task migration dry-run failed: % check(s)', failed_count;
    END IF;
END
$$;
