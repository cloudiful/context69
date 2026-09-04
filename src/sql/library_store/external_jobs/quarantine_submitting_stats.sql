-- Count uncertain `submitting` Docling rows by quarantine eligibility
-- (issue #118 phase 4). Rows that do not meet the quarantine criteria are
-- broken down by the exact reason they were skipped, so operators can see
-- what remains `submitting` and why. The buckets partition every
-- `submitting` row exactly once:
--   * `quarantinable`: placeholder remote id ($2 LIKE pattern), older than
--     the grace cutoff ($1), parent item AND task both terminal;
--   * `skipped_non_terminal`: parent item or task is still active;
--   * `skipped_fresh`: terminal parents but newer than the grace cutoff;
--   * `skipped_real_remote`: terminal parents, old enough, but carrying a
--     non-placeholder remote id that needs manual review.
-- `orphaned` counts rows already isolated by a previous quarantine call.
SELECT COALESCE(count(*) FILTER (WHERE job.status = 'submitting'), 0)::BIGINT
       AS "uncertain_submitting_count!",
       COALESCE(count(*) FILTER (
           WHERE job.status = 'submitting'
             AND job.remote_task_id LIKE $2
             AND job.submitted_at < $1
             AND item.status IN ('succeeded', 'failed', 'cancelled')
             AND task.status IN ('succeeded', 'failed', 'cancelled')
       ), 0)::BIGINT AS "quarantinable_count!",
       COALESCE(count(*) FILTER (
           WHERE job.status = 'submitting'
             AND (item.status NOT IN ('succeeded', 'failed', 'cancelled')
               OR task.status NOT IN ('succeeded', 'failed', 'cancelled'))
       ), 0)::BIGINT AS "skipped_non_terminal_count!",
       COALESCE(count(*) FILTER (
           WHERE job.status = 'submitting'
             AND item.status IN ('succeeded', 'failed', 'cancelled')
             AND task.status IN ('succeeded', 'failed', 'cancelled')
             AND job.submitted_at >= $1
       ), 0)::BIGINT AS "skipped_fresh_count!",
       COALESCE(count(*) FILTER (
           WHERE job.status = 'submitting'
             AND item.status IN ('succeeded', 'failed', 'cancelled')
             AND task.status IN ('succeeded', 'failed', 'cancelled')
             AND job.submitted_at < $1
             AND job.remote_task_id NOT LIKE $2
       ), 0)::BIGINT AS "skipped_real_remote_count!",
       COALESCE(count(*) FILTER (WHERE job.status = 'orphaned'), 0)::BIGINT
       AS "orphaned_count!"
FROM context69.task_external_jobs job
LEFT JOIN context69.task_items item ON item.id = job.item_id
LEFT JOIN context69.tasks task ON task.id = item.task_id
WHERE job.provider = 'docling'
  AND job.status IN ('submitting', 'orphaned');
