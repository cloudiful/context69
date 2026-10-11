-- Claims one batch of rows that still hold the legacy plaintext
-- representation, for the startup sealing backfill.
--
-- Every marker-zero row is claimed, including one whose `key_version` is
-- anomalous, so the run meets it and refuses the batch instead of stranding it
-- outside the sweep. Rows already sealed are not claimed at all: the run never
-- opens, validates, or rewrites an existing ciphertext, so a row sealed under a
-- different key is left exactly as it is.
--
-- Ordering is by key and `FOR UPDATE` deliberately has no `SKIP LOCKED`. A
-- competing startup blocks on the first row of the ordered worklist until the
-- holder commits, instead of seeing a false-empty batch and reporting a sweep
-- that did not happen. Under `READ COMMITTED` the next batch is re-evaluated
-- after each commit, so a row the winner sealed is simply not offered again.
SELECT key,
       value,
       purpose,
       key_version,
       ciphertext_version
FROM context69.internal_secrets
WHERE ciphertext_version = 0
ORDER BY key
LIMIT $1
FOR UPDATE;