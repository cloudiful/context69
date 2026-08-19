-- Insert a recovery audit row for an admin-initiated Docling recovery.
-- Called from the task maintenance service after the new external job has
-- been inserted and Docling has returned the fresh remote id.
INSERT INTO context69.task_docling_recovery_audit (
    task_id,
    item_id,
    actor_user_id,
    actor_login_name,
    reason,
    old_external_job_id,
    old_remote_task_id,
    old_remote_status,
    old_submission_count,
    new_external_job_id,
    new_remote_task_id,
    new_submission_count
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
RETURNING id
