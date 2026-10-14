-- Settles one claimed cleanup row with compare-and-swap on the claim token.
--
-- $1 id, $2 status, $3 outcome, $4 last_error, $5 next_attempt_at,
-- $6 claim_token. Only the holder of the claim token may settle; a stale
-- claimer matches zero rows. Terminal statuses (done/unsupported/failed)
-- stamp finished_at; non-terminal (cancelling/awaiting_terminal/pending)
-- clears the claim so the next tick can reclaim after next_attempt_at.
UPDATE context69.task_docling_remote_cleanup
SET status = $2,
    outcome = $3,
    last_error = $4,
    next_attempt_at = $5,
    finished_at = CASE WHEN $2 IN ('done', 'unsupported', 'failed') THEN now() ELSE NULL END,
    claim_token = NULL,
    claim_until = NULL,
    updated_at = now()
WHERE id = $1
  AND claim_token = $6
RETURNING id;
