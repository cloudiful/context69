-- Frees the rebuild barrier of a cancelled owner (issue 1212 P3).
--
-- Cancellation is not a failure: the cancelled work must never complete, and a
-- late page or completion of it must write nothing. Clearing the owner, the
-- cursor and the pause flag while bumping the epoch invalidates every stale
-- attempt at once. Both running and failed barriers are cleared, because a
-- failed barrier still blocks target editing.
--
-- `$1` narrows the clear to the items of one cancelled task. `NULL` is the bulk
-- cancellation: every barrier whose recorded owner can no longer write is
-- freed. `$2` restricts the clear to a failed barrier for the explicit discard
-- of a terminal rebuild.
--
-- `$3` selects the other mode: the clear of one exact owned barrier. It
-- matches only the barrier that records this owner item, this grant token and
-- this target collection, so a refused attempt frees its own barrier and
-- never the barrier of a newer owner. `$4` is the grant token and `$5` the
-- recorded collection of that exact clear.
UPDATE context69.search_registry_epoch
SET rebuild_state = 'idle',
    rebuild_owner_item = NULL,
    rebuild_owner_token = NULL,
    rebuild_fingerprint = NULL,
    rebuild_collection = NULL,
    rebuild_started_at = NULL,
    rebuild_error = NULL,
    rebuild_pause_requested = FALSE,
    rebuild_cursor_document_id = NULL,
    rebuild_cursor_chunk_index = NULL,
    rebuild_processed_chunks = 0,
    rebuild_total_chunks = 0,
    -- The recorded target is released, so no attempt may treat it as a
    -- collection it created any more.
    rebuild_target_created = FALSE,
    epoch = context69.search_registry_epoch.epoch + 1,
    updated_at = now()
WHERE id = 1
  AND rebuild_state IN ('running', 'failed')
  AND (
        (
            $3::uuid IS NOT NULL
            AND rebuild_owner_item = $3
            AND rebuild_owner_token = $4
            AND rebuild_collection = $5
        )
        OR (
            $3::uuid IS NULL
            AND (
                (
                    $1::uuid IS NOT NULL
                    AND rebuild_owner_item IN (
                        SELECT item.id
                        FROM context69.task_items item
                        WHERE item.task_id = $1
                    )
                    AND (NOT $2::boolean OR rebuild_state = 'failed')
                )
                OR (
                    $1::uuid IS NULL
                    AND NOT EXISTS (
                        SELECT 1
                        FROM context69.task_items item
                        WHERE item.id = rebuild_owner_item
                          AND item.status = 'running'
                          AND item.lease_token = rebuild_owner_token
                    )
                )
            )
        )
    )
RETURNING epoch
