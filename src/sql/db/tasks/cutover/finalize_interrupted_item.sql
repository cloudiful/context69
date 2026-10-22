-- Finalizes one unlinked, expired running item at cutover (issue 1020 P4A).
--
-- Only a running item the library does not execute, whose domain mirror lease
-- is gone, is closed. The item fails with the lease stage and the cutover
-- error; the open domain attempt becomes interrupted with a finished
-- timestamp; the mirror is cleared. The row is never requeued and never
-- replayed, and no segment is produced: a later retry is an explicit user
-- action that opens its own segment under the existing policy. The failure
-- follows the domain retry policy of a lease failure - an item with attempt
-- budget left stays retryable, an exhausted one does not - exactly as
-- `recover_expired_item.sql` decides it, minus the automatic requeue a live
-- spoiler would have to own.
--
-- `$1` is the item and `$2` the cutover error message.
WITH target AS (
    SELECT item.id, item.task_id, item.attempt_count
    FROM context69.task_items item
    WHERE item.id = $1
      AND item.status = 'running'
      AND item.execution_task_id IS NULL
      AND (item.lease_until IS NULL OR item.lease_until <= clock_timestamp())
    FOR UPDATE
),
interrupted AS (
    UPDATE context69.task_attempts attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = $2,
        finished_at = now()
    FROM target
    WHERE attempt.item_id = target.id
      AND attempt.finished_at IS NULL
    RETURNING attempt.id
),
finalized AS (
    UPDATE context69.task_items item
    SET status = 'failed',
        failure_stage = 'lease',
        error_message = $2,
        retryable = (target.attempt_count < 5),
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        lease_token = NULL,
        lease_until = NULL,
        finished_at = now(),
        updated_at = now()
    FROM target
    WHERE item.id = target.id
    RETURNING item.task_id, item.retryable
)
SELECT task_id,
       retryable AS "retryable!",
       (SELECT count(*) FROM interrupted) AS "interrupted!"
FROM finalized
