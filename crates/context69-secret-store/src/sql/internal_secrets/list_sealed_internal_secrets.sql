-- Enumerates the sealed keys one purpose owns at one master-key version. This
-- is the rewrap worklist for master-key recovery and rotation: it returns key
-- names only, never a value, so producing the list cannot disclose a secret.
--
-- The key version is part of the filter, which is what makes a run resumable.
-- A row that a previous run already re-sealed sits at the target version, so it
-- is simply not enumerated again; a row the source key still opens is. The
-- filter therefore also keeps every not-yet-rewrapped row readable with the
-- outgoing key until the deployment switches over, and it never deletes
-- anything.
--
-- `ciphertext_version = 1` excludes the legacy plaintext rows: those hold no
-- frame to open, so they are the backfill's worklist, not the rewrap's.
-- Ordering makes the run deterministic.
SELECT key
FROM context69.internal_secrets
WHERE purpose = $1
  AND key_version = $2
  AND ciphertext_version = 1
ORDER BY key;
