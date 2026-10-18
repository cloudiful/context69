SELECT id,
       failure_stage,
       status,
       failed_count,
       error_summary
FROM context69.tasks
WHERE id = ANY($1)
