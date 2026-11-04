ALTER TABLE context69.task_items
    ADD COLUMN execution_failure_count INTEGER NOT NULL DEFAULT 0
    CHECK (execution_failure_count BETWEEN 0 AND 5);

-- Backfill BEFORE normalizing the claim ordinal: work that the old
-- attempt_count already treated as exhausted keeps a spent failure budget, so
-- it is never resurrected by the new counter. Other rows start at zero because
-- old history cannot distinguish legitimate waits from failures and resets
-- erased that information; this starts a fresh budget rather than fabricating
-- failure history.
UPDATE context69.task_items
SET execution_failure_count = 5
WHERE status IN ('queued', 'waiting')
  AND attempt_count >= 5;

-- attempt_count is only the monotonic claim ordinal from now on. Existing
-- duplicate audit rows are preserved and never rewritten or deleted; raising
-- the item to the highest attempt already recorded prevents future history
-- from reusing an ordinal that an existing task_attempts row owns.
UPDATE context69.task_items AS item
SET attempt_count = GREATEST(
        item.attempt_count,
        COALESCE((
            SELECT max(attempt.attempt)
            FROM context69.task_attempts AS attempt
            WHERE attempt.item_id = item.id
        ), 0)
    );

-- Recreate the task-event notification function from its full current
-- definition, adding only the new internal column to the unchanged-field
-- update suppression list so a counter-only change still notifies.
CREATE OR REPLACE FUNCTION context69.notify_task_event() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    v_task_id uuid;
    v_item_id uuid;
    v_status text;
    v_updated_at timestamptz;
BEGIN
    IF TG_TABLE_NAME = 'tasks' THEN
        IF TG_OP = 'DELETE' THEN
            v_task_id := OLD.id;
            v_item_id := NULL;
            v_status := OLD.status;
            v_updated_at := OLD.updated_at;
        ELSE
            v_task_id := NEW.id;
            v_item_id := NULL;
            v_status := NEW.status;
            v_updated_at := NEW.updated_at;
        END IF;
        PERFORM pg_notify(
            'task_events',
            jsonb_build_object(
                'task_id', v_task_id,
                'item_id', v_item_id,
                'status', v_status,
                'updated_at', v_updated_at
            )::text
        );
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    ELSIF TG_TABLE_NAME = 'task_items' THEN
        IF TG_OP = 'DELETE' THEN
            v_task_id := OLD.task_id;
            v_item_id := OLD.id;
            v_status := OLD.status;
            v_updated_at := OLD.updated_at;
            PERFORM pg_notify(
                'task_events',
                jsonb_build_object(
                    'task_id', v_task_id,
                    'item_id', v_item_id,
                    'status', v_status,
                    'updated_at', v_updated_at
                )::text
            );
            RETURN OLD;
        ELSIF TG_OP = 'INSERT' THEN
            v_task_id := NEW.task_id;
            v_item_id := NEW.id;
            v_status := NEW.status;
            v_updated_at := NEW.updated_at;
            PERFORM pg_notify(
                'task_events',
                jsonb_build_object(
                    'task_id', v_task_id,
                    'item_id', v_item_id,
                    'status', v_status,
                    'updated_at', v_updated_at
                )::text
            );
            RETURN NEW;
        ELSE
            -- UPDATE: suppress pure-heartbeat churn (only lease_until and/or
            -- updated_at changed). Every other column is compared; any
            -- business change (status, stage, lease_token revocation,
            -- failure fields, file/payload, waiting fields, finished_at,
            -- etc.) still notifies.
            IF NEW.task_id IS NOT DISTINCT FROM OLD.task_id
                AND NEW.ordinal IS NOT DISTINCT FROM OLD.ordinal
                AND NEW.payload IS NOT DISTINCT FROM OLD.payload
                AND NEW.status IS NOT DISTINCT FROM OLD.status
                AND NEW.resource_id IS NOT DISTINCT FROM OLD.resource_id
                AND NEW.failure_stage IS NOT DISTINCT FROM OLD.failure_stage
                AND NEW.error_message IS NOT DISTINCT FROM OLD.error_message
                AND NEW.attempt_count IS NOT DISTINCT FROM OLD.attempt_count
                AND NEW.execution_failure_count IS NOT DISTINCT FROM OLD.execution_failure_count
                AND NEW.retryable IS NOT DISTINCT FROM OLD.retryable
                AND NEW.lease_token IS NOT DISTINCT FROM OLD.lease_token
                AND NEW.started_at IS NOT DISTINCT FROM OLD.started_at
                AND NEW.finished_at IS NOT DISTINCT FROM OLD.finished_at
                AND NEW.stage IS NOT DISTINCT FROM OLD.stage
                AND NEW.waiting_reason IS NOT DISTINCT FROM OLD.waiting_reason
                AND NEW.dependency_key IS NOT DISTINCT FROM OLD.dependency_key
                AND NEW.next_attempt_at IS NOT DISTINCT FROM OLD.next_attempt_at
                AND NEW.file_id IS NOT DISTINCT FROM OLD.file_id
                AND NEW.input_storage_object_id IS NOT DISTINCT FROM OLD.input_storage_object_id
                AND NEW.waiting_since IS NOT DISTINCT FROM OLD.waiting_since
                AND NEW.created_at IS NOT DISTINCT FROM OLD.created_at
            THEN
                RETURN NEW;
            END IF;
            v_task_id := NEW.task_id;
            v_item_id := NEW.id;
            v_status := NEW.status;
            v_updated_at := NEW.updated_at;
            PERFORM pg_notify(
                'task_events',
                jsonb_build_object(
                    'task_id', v_task_id,
                    'item_id', v_item_id,
                    'status', v_status,
                    'updated_at', v_updated_at
                )::text
            );
            RETURN NEW;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
