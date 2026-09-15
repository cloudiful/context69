-- Task event bus (issue 405 Task E1): PG NOTIFY triggers for tasks/task_items.
--
-- Channel `task_events` carries a minimal JSON payload limited to
-- task_id/item_id/status/updated_at so every subscriber refetches the full
-- row after wake (best-effort "subscribe + full sync once + incremental"
-- semantics). Payload stays far below the 8000-byte pg_notify limit.
--
-- Coverage: AFTER INSERT/UPDATE/DELETE on context69.tasks and
-- context69.task_items fires for every Rust and pure-SQL writer, including
-- cancel (cancel.sql + cancel_items.sql + recompute.sql), trash/restore
-- (deleted_at updates), clear/delete_trashed (DELETE with CASCADE),
-- maintain_claim_state (exhausted/expired/recomputed), and the poll path
-- that follows update_external_job (wait/progress/finish + recompute).
-- A dedicated trigger on context69.task_external_jobs covers
-- update_external_job directly (INSERT/UPDATE only, no DELETE to avoid
-- CASCADE lookup races); it resolves the owning task_id via task_items and
-- emits the external-job status in the same 4-field envelope.
--
-- Heartbeat throttle: task_items heartbeat (heartbeat_item.sql) only touches
-- lease_until + updated_at every 30s per running item. The item trigger
-- suppresses updates where every column except lease_until/updated_at is
-- unchanged, so heartbeat storms never emit. All other updates (status,
-- stage, lease_token changes such as expired-lease revocation, counts,
-- failure fields, etc.) still notify. External-job poll reservations
-- (claim_poll_reservation.sql, last_polled_at/next_poll_at churn) are
-- throttled the same way: INSERT/UPDATE notifies only when status,
-- remote_status, remote_task_id, or error_message changed.

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

CREATE OR REPLACE FUNCTION context69.notify_task_external_job_event() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
DECLARE
    v_task_id uuid;
BEGIN
    IF TG_OP = 'INSERT' THEN
        SELECT task_id INTO v_task_id
        FROM context69.task_items
        WHERE id = NEW.item_id;
        PERFORM pg_notify(
            'task_events',
            jsonb_build_object(
                'task_id', v_task_id,
                'item_id', NEW.item_id,
                'status', NEW.status,
                'updated_at', NEW.updated_at
            )::text
        );
        RETURN NEW;
    ELSE
        -- UPDATE: throttle poll-reservation churn that only bumps
        -- last_polled_at/next_poll_at/updated_at. Any status, remote_status,
        -- remote_task_id, or error_message change still notifies, which is
        -- exactly the update_external_job path.
        IF NEW.status IS NOT DISTINCT FROM OLD.status
            AND NEW.remote_status IS NOT DISTINCT FROM OLD.remote_status
            AND NEW.remote_task_id IS NOT DISTINCT FROM OLD.remote_task_id
            AND NEW.error_message IS NOT DISTINCT FROM OLD.error_message
        THEN
            RETURN NEW;
        END IF;
        SELECT task_id INTO v_task_id
        FROM context69.task_items
        WHERE id = NEW.item_id;
        PERFORM pg_notify(
            'task_events',
            jsonb_build_object(
                'task_id', v_task_id,
                'item_id', NEW.item_id,
                'status', NEW.status,
                'updated_at', NEW.updated_at
            )::text
        );
        RETURN NEW;
    END IF;
END;
$$;

DROP TRIGGER IF EXISTS trg_tasks_notify ON context69.tasks;
CREATE TRIGGER trg_tasks_notify
    AFTER INSERT OR UPDATE OR DELETE ON context69.tasks
    FOR EACH ROW
    EXECUTE FUNCTION context69.notify_task_event();

DROP TRIGGER IF EXISTS trg_task_items_notify ON context69.task_items;
CREATE TRIGGER trg_task_items_notify
    AFTER INSERT OR UPDATE OR DELETE ON context69.task_items
    FOR EACH ROW
    EXECUTE FUNCTION context69.notify_task_event();

DROP TRIGGER IF EXISTS trg_task_external_jobs_notify ON context69.task_external_jobs;
CREATE TRIGGER trg_task_external_jobs_notify
    AFTER INSERT OR UPDATE ON context69.task_external_jobs
    FOR EACH ROW
    EXECUTE FUNCTION context69.notify_task_external_job_event();
