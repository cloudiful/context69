INSERT INTO context69.tasks
    (id, user_id, group_id, kind, status, group_path, total_count,
     queued_count, running_count, succeeded_count, failed_count,
     cancelled_count, failure_stage, error_summary)
VALUES ($1, $2, $3, 'file_batch', $4, 'test/retire-migration', 1,
        0, 0, 0, 1, 0, $5, 'kept summary')
