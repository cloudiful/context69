SELECT
    (
        SELECT count(*)
          FROM context69.documents document
         WHERE document.metadata_json ? 'extraction'
    ) AS "affected_documents!",
    (
        SELECT count(*)
          FROM context69.task_items item
          JOIN context69.tasks task ON task.id = item.task_id
         WHERE item.status IN ('queued', 'running', 'waiting')
           AND task.status IN ('queued', 'running', 'waiting')
    ) AS "blocking_task_items!",
    (
        SELECT count(*)
          FROM context69.task_docling_remote_jobs job
         WHERE job.status IN ('pending', 'running')
    ) AS "blocking_remote_jobs!"
