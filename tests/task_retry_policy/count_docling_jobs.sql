-- The remote conversion jobs one item submitted.
SELECT count(*) AS "count!"
FROM context69.task_docling_remote_jobs
WHERE item_id = $1
