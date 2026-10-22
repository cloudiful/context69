SELECT count(*) FILTER (WHERE status = 'queued')::bigint AS "queued!",
       count(*) FILTER (WHERE status = 'running')::bigint AS "running!",
       count(*) FILTER (WHERE status = 'waiting')::bigint AS "waiting!",
       count(*) FILTER (WHERE status = 'succeeded')::bigint AS "succeeded!",
       count(*) FILTER (WHERE status = 'failed')::bigint AS "failed!",
       count(*) FILTER (WHERE status = 'cancelled')::bigint AS "cancelled!"
  FROM context69.task_items
 WHERE task_id = $1
