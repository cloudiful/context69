-- Point-in-time snapshot of remaining artifact references (issue #975 P6,
-- total scope corrected in P8).
--
-- The total counts every artifact row, including references under a
-- soft-deleted parent: those rows and their checkpoint bytes still exist and
-- must not be reported as zero. The application may create new references while
-- it is online, so a zero here is a snapshot, never a rollback certificate or a
-- claim about the deployed schema and migration ledger.
SELECT count(*)::bigint AS "total!",
       count(*) FILTER (WHERE artifact.kind = 'download')::bigint AS "download!",
       count(*) FILTER (WHERE artifact.kind = 'sections')::bigint AS "sections!",
       count(*) FILTER (
           WHERE item.lease_token IS NOT NULL OR item.lease_until IS NOT NULL
       )::bigint AS "active!",
       count(*) FILTER (
           WHERE item.lease_token IS NULL
             AND item.lease_until IS NULL
             AND item.status IN ('succeeded', 'failed', 'cancelled')
       )::bigint AS "terminal!",
       count(*) FILTER (WHERE task.deleted_at IS NOT NULL)::bigint AS "soft_deleted!"
FROM context69.task_item_artifacts AS artifact
JOIN context69.task_items AS item ON item.id = artifact.item_id
JOIN context69.tasks AS task ON task.id = item.task_id
