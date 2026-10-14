-- Bounded recent cleanup rows for the admin list (safe projection only).
--
-- Never exposes endpoint_base_url: the UI distinguishes states without
-- learning which fork instance a remote lives on. Ordered newest first,
-- bounded by $1 (the list limit). Includes sanitized outcome/last_error only;
-- raw remote bodies are never stored here.
SELECT
    id,
    remote_job_id,
    task_id,
    item_id,
    remote_task_id,
    reason,
    status,
    outcome,
    attempt_count,
    next_attempt_at,
    created_at,
    updated_at,
    finished_at,
    cleanup_deadline_at,
    last_error
FROM context69.task_docling_remote_cleanup
ORDER BY created_at DESC, id DESC
LIMIT $1;
