-- Exact total for the same persisted-column predicates as list_page.sql.
SELECT COUNT(*)::BIGINT AS "count!"
FROM context69.task_docling_remote_cleanup
WHERE ($1::text IS NULL OR status = $1)
  AND ($2::text IS NULL OR outcome = $2)
  AND ($3::text IS NULL OR remote_task_id = $3);
