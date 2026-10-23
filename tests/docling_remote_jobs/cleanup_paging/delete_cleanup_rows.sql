-- Removes only the rows a paging regression seeded for its own task.
DELETE FROM context69.task_docling_remote_cleanup
WHERE task_id = $1;
