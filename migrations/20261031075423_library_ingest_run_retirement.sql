-- Terminal retirement of an unresolved library ingest run (issue 1138 P18).
--
-- One durable run row is kept per item execution generation instead of one row
-- per item, so a retried item can allocate a fresh generation while the
-- historical rows stay readable. At most one row per item stays current
-- (`retired_at IS NULL`), and only a current row may be adopted or resolved.
--
-- A terminal item status is the retirement authority: every producer already
-- holds the item row, so a trigger on that transition performs the retirement
-- in the same transaction. The trigger moves a staging registry entry to its
-- tombstone form (staged ids become retired ids, reservation shrinks to the
-- tombstone count, epoch bumped once) and marks the run retired without ever
-- deleting history. A run that was never admitted retires without touching the
-- registry, and a switched or resolved run is a success and is never
-- abandoned.

ALTER TABLE context69.library_ingest_runs
    ADD COLUMN retired_at TIMESTAMPTZ;

ALTER TABLE context69.library_ingest_runs
    DROP CONSTRAINT library_ingest_runs_pkey;

ALTER TABLE context69.library_ingest_runs
    ADD CONSTRAINT library_ingest_runs_pkey PRIMARY KEY (item_id, generation);

CREATE UNIQUE INDEX idx_library_ingest_runs_current_item
    ON context69.library_ingest_runs (item_id)
    WHERE retired_at IS NULL;

CREATE INDEX idx_library_ingest_runs_item_history
    ON context69.library_ingest_runs (item_id, generation);

-- Retires the current unresolved run of one item.
--
-- The caller already holds the item row (a status UPDATE or the backfill below),
-- so this helper takes only the remaining locks in the canonical order: epoch,
-- then registry entry, then the run row. The run row is read without a lock
-- before the epoch because the held item row serializes every claim, adopt and
-- switch of the same item.
CREATE OR REPLACE FUNCTION context69.retire_unresolved_library_ingest_run(p_item_id UUID)
RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    current_run context69.library_ingest_runs%ROWTYPE;
    entry_phase TEXT;
BEGIN
    SELECT *
    INTO current_run
    FROM context69.library_ingest_runs
    WHERE item_id = p_item_id
      AND retired_at IS NULL;

    IF NOT FOUND OR current_run.resolved_at IS NOT NULL THEN
        RETURN;
    END IF;

    PERFORM 1
    FROM context69.search_registry_epoch
    WHERE id = 1
    FOR UPDATE;

    SELECT phase
    INTO entry_phase
    FROM context69.search_registry_entries
    WHERE run_id = current_run.batch_id
    FOR UPDATE;

    IF FOUND THEN
        IF entry_phase = 'switched' THEN
            -- The switch committed the new version; its run is a success even
            -- when the item terminates before the caller marks it resolved.
            RETURN;
        END IF;

        IF entry_phase = 'staging' THEN
            UPDATE context69.search_registry_entries
            SET phase = 'retired',
                retired_ids = retired_ids || staged_ids,
                staged_ids = '{}'::uuid[],
                reserved_ids = cardinality(retired_ids || staged_ids)
            WHERE run_id = current_run.batch_id
              AND phase = 'staging';

            IF FOUND THEN
                UPDATE context69.search_registry_epoch
                SET epoch = epoch + 1,
                    updated_at = now()
                WHERE id = 1;
            END IF;
        END IF;
    END IF;

    UPDATE context69.library_ingest_runs
    SET retired_at = now(),
        updated_at = now()
    WHERE item_id = current_run.item_id
      AND generation = current_run.generation
      AND retired_at IS NULL;
END;
$$;

CREATE OR REPLACE FUNCTION context69.retire_library_ingest_run_on_terminal_item()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM context69.retire_unresolved_library_ingest_run(NEW.id);
    RETURN NULL;
END;
$$;

CREATE TRIGGER task_items_retire_library_ingest_run
AFTER UPDATE OF status ON context69.task_items
FOR EACH ROW
WHEN (
    NEW.status IN ('failed', 'cancelled')
    AND NEW.status IS DISTINCT FROM OLD.status
)
EXECUTE FUNCTION context69.retire_library_ingest_run_on_terminal_item();

-- Reconcile rows that already reached a terminal status before the trigger
-- existed. Each item is locked first, then the helper follows the canonical
-- lock order; items are visited in a stable order.
DO $$
DECLARE
    terminal_item RECORD;
BEGIN
    FOR terminal_item IN
        SELECT item.id
        FROM context69.task_items item
        JOIN context69.library_ingest_runs run
          ON run.item_id = item.id
         AND run.retired_at IS NULL
         AND run.resolved_at IS NULL
        WHERE item.status IN ('failed', 'cancelled')
        ORDER BY item.id
    LOOP
        PERFORM 1
        FROM context69.task_items
        WHERE id = terminal_item.id
        FOR UPDATE;

        PERFORM context69.retire_unresolved_library_ingest_run(terminal_item.id);
    END LOOP;
END;
$$;
